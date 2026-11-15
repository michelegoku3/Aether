#include "pch.h"
#include "hooks/wire/AchievementUi.h"

#include <algorithm>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <system_error>
#include <utility>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "core/NetPacketAbi.h"
#include "core/Workers.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/wire/AchievementSchemaExport.h"
#include "hooks/wire/AchievementUserState.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/SchemaCache.h"
#include "scripting/LuaData.h"
#include "steam_messages.pb.h"
#include "utils/GameNameResolver.h"
#include "utils/WinFile.h"

namespace ac::achieve {
namespace {

constexpr const char* kModule = "AchieveUI";
constexpr std::int32_t kNoChange = -1;

// A job id CJobMgr never allocates: aiming a reply at it makes the client drop
// the frame and leave the real job pending.
constexpr std::uint64_t kDroppedJobId = 1;

// A held reply that still has no data after this long is released as Valve sent
// it, well before the client's job would time out.
constexpr auto kHoldTimeout = std::chrono::seconds(8);

// Bounds. A page open asks for one app, so these only have to cover the apps a
// session actually browses; the oldest entry leaves first.
constexpr std::size_t kMaxSchemas = 64;
constexpr std::size_t kMaxPending = 64;
constexpr std::size_t kMaxStates = 64;
constexpr std::size_t kMaxStatsSchemas = 64;
constexpr std::size_t kMaxHeld = 16;
constexpr std::size_t kMaxOutbox = 16;

State& S() { return g_state.achievements.ui; }

std::uint32_t AccountIdFromSteamId(std::uint64_t steamId) {
    constexpr std::uint64_t kIndividualBase = 76561197960265728ull;
    if (steamId < kIndividualBase)
        return 0;
    const std::uint64_t id = steamId - kIndividualBase;
    return id > 0xFFFFFFFFull ? 0 : static_cast<std::uint32_t>(id);
}

std::string StatsDir() {
    return g_state.steamInstallPath.empty() ? std::string() : g_state.steamInstallPath + "\\appcache\\stats";
}

std::string UserBinPath(std::uint32_t accountId, std::uint32_t appId) {
    return StatsDir() + "\\UserGameStats_" + std::to_string(accountId) + "_" + std::to_string(appId) + ".bin";
}

std::string StatsSchemaPath(std::uint32_t appId) {
    return StatsDir() + "\\UserGameStatsSchema_" + std::to_string(appId) + ".bin";
}

// Size and write time of a stats file. The worker uses them to tell whether the
// cached parse still matches the file Steam wrote.
struct FileStamp {
    bool present = false;
    std::uintmax_t size = 0;
    std::int64_t writeTime = 0;
};

FileStamp StatFile(const std::string& path) {
    FileStamp stamp;
    std::error_code ec;
    const auto fsPath = winfile::ToPath(path);
    stamp.size = std::filesystem::file_size(fsPath, ec);
    if (ec)
        return stamp;
    const auto time = std::filesystem::last_write_time(fsPath, ec);
    if (ec)
        return stamp;
    stamp.present = true;
    stamp.writeTime = static_cast<std::int64_t>(time.time_since_epoch().count());
    return stamp;
}

bool ReadFileBytes(const std::string& path, std::vector<std::uint8_t>& out) {
    std::ifstream file(winfile::ToPath(path), std::ios::binary);
    if (!file.is_open())
        return false;
    out.assign(std::istreambuf_iterator<char>(file), std::istreambuf_iterator<char>());
    return true;
}

// Aether's own copy of the same stats file, or "" when AetherDesk is absent.
std::string SnapshotBinPath(std::uint32_t accountId, std::uint32_t appId) {
    const std::string dir = ac::backup::io::BackupDirForApp(appId);
    if (dir.empty())
        return {};
    return dir + "\\UserGameStats_" + std::to_string(accountId) + "_" + std::to_string(appId) + ".bin";
}

// Reads Aether's own copy of the same stats file (AchievementBackup.h). Steam
// rewrites the client cache for an app the account does not own, and what it
// writes can be empty; this copy is what outlives that, so the page reads both
// and lets MergeUserState decide per key.
bool ReadSnapshotState(std::uint32_t accountId, std::uint32_t appId, UserState& out) {
    const std::string path = SnapshotBinPath(accountId, appId);
    if (path.empty())
        return false;
    std::vector<std::uint8_t> bytes;
    if (!ReadFileBytes(path, bytes) || bytes.empty())
        return false;
    return ParseUserState(bytes.data(), bytes.size(), out);
}

// Puts Aether's copy back where the client reads it.
//
// The game page's achievements block is built from this file, not from the reply
// this module rewrites: a page whose file Steam emptied cannot be repaired by
// answering, because the client caches what the file says and draws the section
// from it. The staging file keeps the replacement atomic, so the client never
// reads a half-written copy.
bool RestoreSnapshotFile(std::uint32_t accountId, std::uint32_t appId) {
    const std::string source = SnapshotBinPath(accountId, appId);
    const std::string target = UserBinPath(accountId, appId);
    if (source.empty() || target.empty() || source == target)
        return false;
    const std::string staging = ac::backup::io::UniqueStagingPath(target);
    std::error_code ec;
    std::filesystem::copy_file(winfile::ToPath(source), winfile::ToPath(staging),
                               std::filesystem::copy_options::overwrite_existing, ec);
    if (ec) {
        AC_LOG_DEBUG(kModule, "App %u: recovered stats cache not staged (%d).", appId, ec.value());
        return false;
    }
    if (!ac::backup::io::AtomicReplace(staging, target)) {
        std::filesystem::remove(winfile::ToPath(staging), ec);
        return false;
    }
    return true;
}

std::uint64_t StateKey(std::uint32_t appId, std::uint32_t accountId) {
    return (static_cast<std::uint64_t>(appId) << 32) | accountId;
}

void SetReason(const char* reason) {
    {
        std::lock_guard<std::mutex> lock(S().mutex);
        S().lastReason = reason;
    }
    // status.json is a snapshot the writer only refreshes on request, and these
    // counters are the only way to see what the page decided without a log.
    ac::status::Write();
}

void RememberPending(std::uint64_t jobId, const Pending& pending) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    if (!s.pending.emplace(jobId, pending).second)
        return;
    s.pendingOrder.push_back(jobId);
    while (s.pendingOrder.size() > kMaxPending) {
        s.pending.erase(s.pendingOrder.front());
        s.pendingOrder.pop_front();
    }
}

bool TakePending(std::uint64_t jobId, Pending& out) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    const auto it = s.pending.find(jobId);
    if (it == s.pending.end())
        return false;
    out = it->second;
    s.pending.erase(it);
    const auto order = std::find(s.pendingOrder.begin(), s.pendingOrder.end(), jobId);
    if (order != s.pendingOrder.end())
        s.pendingOrder.erase(order);
    return true;
}

void StoreSchema(std::uint32_t appId, CapturedSchema schema) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    if (!s.schemas.contains(appId))
        s.schemaOrder.push_back(appId);
    s.schemas[appId] = std::move(schema);
    while (s.schemaOrder.size() > kMaxSchemas) {
        s.schemas.erase(s.schemaOrder.front());
        s.schemaOrder.pop_front();
    }
}

bool CopySchema(std::uint32_t appId, CapturedSchema& out) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    const auto it = s.schemas.find(appId);
    if (it == s.schemas.end())
        return false;
    out = it->second;
    return true;
}

bool CopyState(std::uint64_t key, CachedState& out) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    const auto it = s.states.find(key);
    if (it == s.states.end())
        return false;
    out = it->second;
    return true;
}

bool CopyStatsSchema(std::uint32_t appId, CachedStatsSchema& out) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    const auto it = s.statsSchemas.find(appId);
    if (it == s.statsSchemas.end())
        return false;
    out = it->second;
    return true;
}

void StoreStatsSchema(std::uint32_t appId, CachedStatsSchema schema) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    if (!s.statsSchemas.contains(appId))
        s.statsSchemaOrder.push_back(appId);
    s.statsSchemas[appId] = std::move(schema);
    while (s.statsSchemaOrder.size() > kMaxStatsSchemas) {
        s.statsSchemas.erase(s.statsSchemaOrder.front());
        s.statsSchemaOrder.pop_front();
    }
}

// Builds a complete 147 frame and queues it for delivery by TryDeliver.
bool QueueFrame(const std::string& header, const std::string& body) {
    const std::uint32_t headerLen = static_cast<std::uint32_t>(header.size());
    const std::uint32_t bodyLen = static_cast<std::uint32_t>(body.size());
    if (headerLen > constants::kWireMaxHeaderBytes || bodyLen > constants::kWireMaxBodyBytes)
        return false;
    const std::uint32_t total = sizeof(steam::MsgHdr) + headerLen + bodyLen;
    std::vector<std::uint8_t> packet(total);
    auto* msg = reinterpret_cast<steam::MsgHdr*>(packet.data());
    msg->eMsg = constants::emsg::kServiceMethodResponse | steam::kMsgHdrProtoFlag;
    msg->headerLength = headerLen;
    if (headerLen != 0)
        std::memcpy(packet.data() + sizeof(steam::MsgHdr), header.data(), headerLen);
    if (bodyLen != 0)
        std::memcpy(packet.data() + sizeof(steam::MsgHdr) + headerLen, body.data(), bodyLen);

    std::lock_guard<std::mutex> lock(S().mutex);
    if (S().outbox.size() >= kMaxOutbox)
        return false;
    S().outbox.push_back(std::move(packet));
    return true;
}

// Builds the rebuilt body from the captured schema and the cached local state.
// Returns false with `hold` set when the data is not ready yet, and false with
// `reason` set when the body cannot be produced (answer Valve's instead).
bool BuildBody(std::uint32_t appId, std::uint32_t accountId, const std::string& valveBody, std::string& out,
               const char*& reason, bool& hold) {
    hold = false;
    CapturedSchema schema;
    const bool haveSchema = CopySchema(appId, schema);
    CachedState state;
    const bool haveState = accountId == 0 || (CopyState(StateKey(appId, accountId), state) && state.loaded);

    Input in;
    in.managed = true;
    in.schemaKnown = haveSchema && !schema.entries.empty();
    in.entries = haveSchema ? schema.entries : std::vector<SchemaEntry>{};
    in.groups = haveSchema ? schema.groups : std::vector<std::uint32_t>{};
    in.stateLoaded = haveState;
    if (haveState) {
        in.unlocked = state.unlocked;
        in.progress = state.progress;
    }

    const Decision decision = Decide(in);
    reason = decision.reason;
    if (decision.verdict == Verdict::Hold) {
        hold = true;
        return false;
    }
    if (decision.verdict != Verdict::Rebuild)
        return false;

    CPlayer_GetUserAchievements_Response reply;
    if (!reply.ParseFromArray(valveBody.data(), static_cast<int>(valveBody.size()))) {
        reason = "reply-unparsed";
        return false;
    }
    FillReply(reply, decision);
    out.resize(reply.ByteSizeLong());
    if (!out.empty() && !reply.SerializeToArray(out.data(), static_cast<int>(out.size()))) {
        out.clear();
        reason = "encode";
        return false;
    }

    std::size_t withProgress = 0;
    for (const ReplyEntry& entry : decision.reply.entries) {
        if (entry.progress != 0.0f)
            ++withProgress;
    }
    S().progressValues.store(withProgress, std::memory_order_relaxed);
    return true;
}

void HoldReply(std::uint32_t appId, std::uint32_t accountId, const std::string& header, const std::uint8_t* body,
               std::uint32_t len) {
    HeldReply held;
    held.appId = appId;
    held.accountId = accountId;
    held.header = header;
    if (body && len != 0)
        held.body.assign(reinterpret_cast<const char*>(body), len);
    held.at = std::chrono::steady_clock::now();

    bool overflow = false;
    HeldReply oldest;
    {
        State& s = S();
        std::lock_guard<std::mutex> lock(s.mutex);
        if (s.held.size() >= kMaxHeld) {
            oldest = std::move(s.held.front());
            s.held.erase(s.held.begin());
            overflow = true;
        }
        s.held.push_back(std::move(held));
    }
    // A job cannot be left unanswered: release the oldest as Valve sent it.
    if (overflow)
        QueueFrame(oldest.header, oldest.body);
}

// Rebuilds every held reply for one app whose data has since arrived. Runs on
// the schema/worker thread, never on a Steam network thread in the held case.
void FlushHeld(std::uint32_t appId) {
    std::vector<HeldReply> waiting;
    {
        State& s = S();
        std::lock_guard<std::mutex> lock(s.mutex);
        for (auto it = s.held.begin(); it != s.held.end();) {
            if (it->appId == appId) {
                waiting.push_back(std::move(*it));
                it = s.held.erase(it);
            } else {
                ++it;
            }
        }
    }
    for (HeldReply& held : waiting) {
        std::string body;
        const char* reason = "";
        bool hold = false;
        if (BuildBody(held.appId, held.accountId, held.body, body, reason, hold)) {
            if (QueueFrame(held.header, body)) {
                AC_LOG_DEBUG(kModule, "App %u: held reply rebuilt from local state.", held.appId);
                continue;
            }
        }
        if (hold) {
            State& s = S();
            std::lock_guard<std::mutex> lock(s.mutex);
            if (s.held.size() < kMaxHeld)
                s.held.push_back(std::move(held));
            continue;
        }
        // The data will not get any better: answer with Valve's own reply.
        QueueFrame(held.header, held.body);
    }
}

// Releases held replies whose schema or state never arrived.
void ExpireHeld() {
    const auto now = std::chrono::steady_clock::now();
    std::vector<HeldReply> expired;
    {
        State& s = S();
        std::lock_guard<std::mutex> lock(s.mutex);
        for (auto it = s.held.begin(); it != s.held.end();) {
            if (now - it->at >= kHoldTimeout) {
                expired.push_back(std::move(*it));
                it = s.held.erase(it);
            } else {
                ++it;
            }
        }
    }
    for (const HeldReply& held : expired) {
        QueueFrame(held.header, held.body);
        AC_LOG_DEBUG(kModule, "App %u: data never arrived, released Valve's reply.", held.appId);
    }
}

// Worker body: transcribe the captured schema into the file the overlay and the
// game read (<steam>\appcache\stats\UserGameStatsSchema_<appid>.bin).
//
// The legacy stats path takes its schema from a donor whose account owns the
// game (AchievementModule), so for a managed app the file never appears: the
// client then answers the game with no achievement list at all and Shift+Tab
// shows an empty, disabled tab. The page reply already carries the same
// achievements, so this writes the tree the client would have downloaded.
//
// An existing file is never replaced (SchemaCache::Store): a real download, or
// a schema another donor already produced, stays authoritative.
void PublishSchemaFile(std::uint32_t appId, CapturedSchema schema) {
    if (StatsDir().empty() || !ac::luadata::HasDepot(appId))
        return;
    const std::string path = StatsSchemaPath(appId);
    if (winfile::Exists(path))
        return; // Steam, or an earlier run, already put one there

    SchemaFileInput in;
    in.appId = appId;
    in.version = schema.version;
    in.language = schema.language;
    in.gameName = ac::gamename::ForApp(appId);
    in.entries = std::move(schema.entries);
    const std::size_t count = in.entries.size();
    const std::string bytes = EncodeSchemaFile(in);
    if (bytes.empty())
        return;

    if (!ac::hooks::wire::schemacache::Store(appId, bytes, StatsDir(), ac::backup::io::BackupDirForApp(appId))) {
        AC_LOG_DEBUG(kModule, "App %u: the transcribed schema could not be written.", appId);
        return;
    }
    S().schemaFilesWritten.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();
    AC_LOG_INFO(kModule,
                "App %u: transcribed the achievement schema into appcache\\stats (%zu achievement(s), %zu byte): "
                "the overlay reads it at game start.",
                appId, count, bytes.size());
}

// Worker body: read the stats file and the schema that names its counters,
// publish the parsed result. Disk and KV1 walking run here, never on a Steam
// thread.
void ReadStateNow(std::uint64_t key, std::uint32_t accountId, std::uint32_t appId) {
    CachedState fresh;
    fresh.loaded = true;

    const std::string userPath = UserBinPath(accountId, appId);
    const FileStamp userFile = StatFile(userPath);
    UserState state;
    if (userFile.present && userFile.size > 0) {
        std::vector<std::uint8_t> bytes;
        if (ReadFileBytes(userPath, bytes)) {
            fresh.known = ParseUserState(bytes.data(), bytes.size(), state);
            fresh.size = bytes.size();
            fresh.writeTime = userFile.writeTime;
        }
    }
    if (UserState snapshot; ReadSnapshotState(accountId, appId, snapshot)) {
        // The client builds the page from this file, so put back what Steam
        // emptied before the page answers.
        if (HasUnlockMissingFrom(snapshot, state) && RestoreSnapshotFile(accountId, appId)) {
            AC_LOG_INFO(kModule, "App %u: put %zu unlock(s) back into Steam's stats cache.", appId,
                        snapshot.unlocked.size());
        }
        const std::size_t before = state.unlocked.size();
        MergeUserState(state, snapshot);
        if (state.unlocked.size() > before) {
            AC_LOG_DEBUG(kModule, "App %u: %zu unlock(s) came from Aether's snapshot, not Steam's cache.", appId,
                         state.unlocked.size() - before);
        }
    }

    // Which stat drives an achievement's counter is the game schema's to say.
    // The parse is cached per app and re-run only when Steam rewrites the file.
    CachedStatsSchema statsSchema;
    const bool cached = CopyStatsSchema(appId, statsSchema);
    const FileStamp schemaFile = StatFile(StatsSchemaPath(appId));
    if (schemaFile.present &&
        (!cached || statsSchema.size != schemaFile.size || statsSchema.writeTime != schemaFile.writeTime)) {
        std::vector<std::uint8_t> bytes;
        if (ReadFileBytes(StatsSchemaPath(appId), bytes)) {
            CachedStatsSchema parsed;
            parsed.size = schemaFile.size;
            parsed.writeTime = schemaFile.writeTime;
            ParseStatsSchema(bytes.data(), bytes.size(), parsed.schema);
            AC_LOG_DEBUG(kModule, "App %u: game schema read, %zu achievement(s) with a counter.", appId,
                         parsed.schema.progressStat.size());
            statsSchema = parsed;
            StoreStatsSchema(appId, std::move(parsed));
        } else {
            statsSchema = CachedStatsSchema{};
        }
    }

    for (const auto& [achievementKey, statId] : statsSchema.schema.progressStat) {
        const auto value = state.values.find(statId);
        if (value != state.values.end())
            fresh.progress[achievementKey] = value->second;
    }
    fresh.unlocked = std::move(state.unlocked);

    {
        State& s = S();
        std::lock_guard<std::mutex> lock(s.mutex);
        s.states[key] = std::move(fresh);
        if (s.states.size() > kMaxStates) {
            // Drop the entry that is not this one and was inserted earliest.
            // The map has no order, so a state that is not the current key may
            // leave: the next page open re-reads it.
            for (auto it = s.states.begin(); it != s.states.end(); ++it) {
                if (it->first != key) {
                    s.states.erase(it);
                    break;
                }
            }
        }
        s.refreshing.erase(key);
    }
    // A reply parked waiting for this read can now be answered.
    FlushHeld(appId);
    ac::status::Write();
}

// Schedules a read for (app, account). At most one is in flight per pair, so a
// page refresh loop cannot queue unbounded work.
void RequestStateRead(std::uint32_t appId, std::uint32_t accountId) {
    if (accountId == 0 || g_state.steamInstallPath.empty())
        return;
    const std::uint64_t key = StateKey(appId, accountId);
    {
        State& s = S();
        std::lock_guard<std::mutex> lock(s.mutex);
        if (!s.refreshing.insert(key).second)
            return;
    }
    const bool queued = workers::Submit([key, accountId, appId] { ReadStateNow(key, accountId, appId); });
    if (!queued) {
        // Shutdown: leave the cache cold, the held reply times out.
        std::lock_guard<std::mutex> lock(S().mutex);
        S().refreshing.erase(key);
    }
}

} // namespace

bool Enabled() {
    const auto settings = Settings::Snapshot();
    return settings && settings->achievementsUiService;
}

void RestoreTranscribedSchemas() {
    const std::string root = ac::backup::io::CachedDeskDataDir();
    if (root.empty() || g_state.steamInstallPath.empty())
        return; // no AetherDesk, so nowhere the capture could have been kept
    const std::string statsDir = StatsDir();
    // Win32 matches no wildcard in a directory component, so the walk is one
    // level at a time: every "<backup>\<appid>" folder, then its schema file.
    const std::string pattern = root + "\\backup\\*";
    WIN32_FIND_DATAA fd{};
    HANDLE find = FindFirstFileA(pattern.c_str(), &fd);
    if (find == INVALID_HANDLE_VALUE)
        return; // nothing transcribed yet
    std::size_t restored = 0;
    do {
        if ((fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) == 0)
            continue;
        const std::uint32_t appId = ac::hooks::wire::schemacache::AppIdFromBackupDir(fd.cFileName);
        // The lua file may have been removed since the capture: an app Aether no
        // longer manages is not Aether's to prepare.
        if (appId == 0 || !ac::luadata::HasDepot(appId))
            continue;
        // A cache file Steam wrote itself is authoritative; Restore would leave
        // it alone anyway, this only avoids walking the backup for nothing.
        if (winfile::Exists(StatsSchemaPath(appId)))
            continue;
        if (ac::hooks::wire::schemacache::Restore(appId, statsDir, ac::backup::io::BackupDirForApp(appId))) {
            ++restored;
            AC_LOG_INFO(kModule,
                        "Backup: put the transcribed achievement schema for app %u back into Steam's stats cache.",
                        appId);
        }
    } while (FindNextFileA(find, &fd));
    FindClose(find);
    if (restored != 0)
        ac::status::Write();
}

bool OnSendSchema(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len) {
    if (!Enabled())
        return false;
    CPlayer_GetGameAchievements_Request request;
    if (!body || !request.ParseFromArray(body, static_cast<int>(len)))
        return false;
    S().calls.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();
    Pending pending;
    pending.userCall = false;
    pending.appId = request.appid();
    pending.language = request.language();
    RememberPending(jobId, pending);
    return true;
}

bool OnSendUser(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len) {
    if (!Enabled())
        return false;
    CPlayer_GetUserAchievements_Request request;
    if (!body || !request.ParseFromArray(body, static_cast<int>(len)))
        return false;
    S().calls.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();

    Pending pending;
    pending.userCall = true;
    pending.appId = request.appid();
    pending.steamId = request.steamid();
    RememberPending(jobId, pending);

    // The read starts now so it is usually done by the time the reply lands.
    // The account must come from the request: resolving it from the registry or
    // loginusers.vdf would be disk I/O on a Steam network thread.
    if (ac::luadata::HasDepot(pending.appId))
        RequestStateRead(pending.appId, AccountIdFromSteamId(pending.steamId));
    return true;
}

std::int32_t OnRecvSchema(std::uint64_t jobId, const std::uint8_t* body, std::uint32_t len) {
    Pending pending;
    if (!TakePending(jobId, pending) || pending.userCall)
        return kNoChange;
    CPlayer_GetGameAchievements_Response reply;
    if (!body || !reply.ParseFromArray(body, static_cast<int>(len))) {
        SetReason("schema-unparsed");
        return kNoChange;
    }

    // A hash-only or empty reply must never wipe a good capture: the client
    // asks with hash_only=1 when it only wants to invalidate, and the next real
    // reply carries the entries again.
    if (reply.achievements_size() == 0) {
        SetReason("schema-empty");
        return kNoChange;
    }

    CapturedSchema schema;
    schema.version = reply.schema_version();
    schema.hash = reply.schema_hash();
    schema.language = pending.language;
    schema.entries.reserve(static_cast<std::size_t>(reply.achievements_size()));
    for (const auto& achievement : reply.achievements()) {
        // A key without a name cannot be rendered, and a name without a key
        // cannot be matched against the local bits: skip both.
        if (!achievement.has_internal_key() || !achievement.has_internal_name() || achievement.internal_name().empty())
            continue;
        SchemaEntry entry;
        entry.key = achievement.internal_key();
        entry.groupid = achievement.groupid();
        // The display fields exist for the schema file the overlay reads; the
        // page renders from Valve's own reply and never needs them.
        entry.internalName = achievement.internal_name();
        entry.name = achievement.name();
        entry.description = achievement.description();
        entry.icon = achievement.icon();
        entry.iconGray = achievement.icon_gray();
        entry.hidden = achievement.hidden() != 0;
        schema.entries.push_back(std::move(entry));
    }
    schema.groups.reserve(static_cast<std::size_t>(reply.groups_size()));
    for (const auto& group : reply.groups()) {
        if (group.has_groupid())
            schema.groups.push_back(group.groupid());
    }
    if (schema.entries.empty()) {
        SetReason("schema-empty");
        return kNoChange;
    }

    const std::size_t count = schema.entries.size();
    const std::uint32_t hash = schema.hash;
    const std::int32_t version = schema.version;
    // The overlay's own schema file is written off-thread: a full capture is a
    // few dozen KB and this runs on a Steam network thread.
    CapturedSchema publish = schema;
    StoreSchema(pending.appId, std::move(schema));
    workers::Submit([appId = pending.appId, publish = std::move(publish)]() mutable {
        PublishSchemaFile(appId, std::move(publish));
    });
    S().schemasCaptured.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();
    AC_LOG_DEBUG(kModule, "App %u: captured %zu achievement(s), schema v%d hash %08X.", pending.appId, count, version,
                 hash);
    // Replies waiting on this schema can now be answered.
    FlushHeld(pending.appId);
    // The schema reply is never rewritten: the client renders names, icons and
    // descriptions from it and Aether has nothing better to offer.
    return kNoChange;
}

std::int32_t OnRecvUser(std::uint64_t jobId, const std::uint8_t* header, std::uint32_t headerLen,
                        const std::uint8_t* body, std::uint32_t len, std::uint8_t* outHeader,
                        std::uint32_t outHeaderCap, std::int32_t* outHeaderLen, std::uint8_t* out, std::uint32_t cap) {
    Pending pending;
    if (!TakePending(jobId, pending) || !pending.userCall)
        return kNoChange;
    if (!Enabled()) {
        SetReason("off");
        return kNoChange;
    }
    if (!ac::luadata::HasDepot(pending.appId)) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason("not-managed");
        return kNoChange;
    }
    if (!header || headerLen == 0 || !outHeader || !outHeaderLen) {
        SetReason("header");
        return kNoChange;
    }

    // The CM marks the app denied because the account holds no licence. The page
    // is not denied, it just has no data, so the reply goes out as OK.
    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(header, static_cast<int>(headerLen))) {
        SetReason("header-unparsed");
        return kNoChange;
    }
    hdr.set_eresult(1); // k_EResultOK
    std::string headerBytes;
    if (!hdr.SerializeToString(&headerBytes) || headerBytes.size() > outHeaderCap) {
        SetReason("header-large");
        return kNoChange;
    }

    const std::uint32_t accountId = AccountIdFromSteamId(pending.steamId);
    RequestStateRead(pending.appId, accountId);

    std::string rebuilt;
    const char* reason = "";
    bool hold = false;
    const std::string valveBody = body ? std::string(reinterpret_cast<const char*>(body), len) : std::string();
    if (BuildBody(pending.appId, accountId, valveBody, rebuilt, reason, hold)) {
        if (rebuilt.size() > cap) {
            S().passthrough.fetch_add(1, std::memory_order_relaxed);
            SetReason("too-large");
            AC_LOG_WARN_ONCE(kModule, "Rebuilt reply for app %u needs %zu byte (cap %u); passing Steam's through.",
                             pending.appId, rebuilt.size(), cap);
            return kNoChange;
        }
        std::copy(headerBytes.begin(), headerBytes.end(), outHeader);
        *outHeaderLen = static_cast<std::int32_t>(headerBytes.size());
        if (!rebuilt.empty())
            std::copy(rebuilt.begin(), rebuilt.end(), out);
        S().rebuilt.fetch_add(1, std::memory_order_relaxed);
        SetReason("rebuilt");
        AC_LOG_DEBUG(kModule, "App %u: page answered from local state (%zu byte, %llu entry with progress).",
                     pending.appId, rebuilt.size(),
                     static_cast<unsigned long long>(S().progressValues.load(std::memory_order_relaxed)));
        return static_cast<std::int32_t>(rebuilt.size());
    }
    if (!hold) {
        S().passthrough.fetch_add(1, std::memory_order_relaxed);
        SetReason(reason);
        AC_LOG_DEBUG(kModule, "App %u: page left to Steam (%s).", pending.appId, reason);
        return kNoChange;
    }

    // Hold: keep the real header+body for a later delivery, and aim this copy at
    // a job nobody waits on so the client drops it and keeps the real job open.
    HoldReply(pending.appId, accountId, headerBytes, body, len);
    hdr.set_jobid_target(kDroppedJobId);
    if (!hdr.SerializeToString(&headerBytes) || headerBytes.size() > outHeaderCap) {
        SetReason("header-large");
        return kNoChange;
    }
    std::copy(headerBytes.begin(), headerBytes.end(), outHeader);
    *outHeaderLen = static_cast<std::int32_t>(headerBytes.size());
    if (len != 0) {
        if (len > cap)
            return kNoChange;
        std::copy_n(body, len, out);
    }
    S().heldCount.fetch_add(1, std::memory_order_relaxed);
    SetReason(reason);
    AC_LOG_DEBUG(kModule, "App %u: reply held (%s).", pending.appId, reason);
    return static_cast<std::int32_t>(len);
}

void TryDeliver(void* recvThis, steam::CNetPacket* carrier, void* (*oRecvPkt)(void*, steam::CNetPacket*)) {
    if (!carrier || !oRecvPkt || !abi::netpkt::IsResolved())
        return;

    // Release anything whose data never arrived before choosing a frame.
    ExpireHeld();

    std::vector<std::uint8_t> packet;
    {
        State& s = S();
        std::lock_guard<std::mutex> lock(s.mutex);
        if (s.outbox.empty())
            return;
        packet = std::move(s.outbox.front());
        s.outbox.pop_front();
    }

    // Same barrier as PacketRouter: repointing the carrier is the most
    // destructive write Aether makes. Keep the frame queued when it refuses.
    if (!abi::netpkt::BeginWrite(carrier)) {
        std::lock_guard<std::mutex> lock(S().mutex);
        S().outbox.push_front(std::move(packet));
        return;
    }

    std::uint8_t*& carrierData = abi::netpkt::Data(carrier);
    std::uint32_t& carrierLen = abi::netpkt::Size(carrier);
    std::uint8_t* const origData = carrierData;
    const std::uint32_t origLen = carrierLen;
    carrierData = packet.data();
    carrierLen = static_cast<std::uint32_t>(packet.size());
    oRecvPkt(recvThis, carrier);
    carrierData = origData;
    carrierLen = origLen;

    S().delivered.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();
}

} // namespace ac::achieve
