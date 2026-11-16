#include "pch.h"
#include "hooks/wire/AchievementUi.h"

#include <algorithm>
#include <cstring>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <system_error>
#include <thread>
#include <utility>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "core/NetPacketAbi.h"
#include "core/Workers.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/wire/AchievementPrefetchPolicy.h"
#include "hooks/wire/AchievementSchemaExport.h"
#include "hooks/wire/AchievementUserState.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/PacketRouter.h"
#include "hooks/wire/SchemaCache.h"
#include "scripting/LuaData.h"
#include "steam_messages.pb.h"
#include "utils/GameNameResolver.h"
#include "utils/Strings.h"
#include "utils/WinFile.h"

namespace ac::achieve {
namespace prefetch = ac::hooks::wire::prefetch;
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

// Drops a send that never went out: a failed SendClientServiceCall would
// otherwise leave a pending entry for a reply that cannot arrive.
void ForgetPending(std::uint64_t jobId) {
    State& s = S();
    std::lock_guard<std::mutex> lock(s.mutex);
    if (s.pending.erase(jobId) == 0)
        return;
    const auto order = std::find(s.pendingOrder.begin(), s.pendingOrder.end(), jobId);
    if (order != s.pendingOrder.end())
        s.pendingOrder.erase(order);
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
// An existing file is never replaced unless it is Aether's own transcription
// (SchemaCache::StoreTranscribed): a real Steam download stays authoritative,
// while a game that adds achievements bumps the schema version and the
// transcription follows.
void PublishSchemaFile(std::uint32_t appId, CapturedSchema schema) {
    if (StatsDir().empty() || !ac::luadata::HasDepot(appId))
        return;

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

    const bool existed = winfile::Exists(StatsSchemaPath(appId));
    ac::hooks::wire::schemacache::SchemaWrite applied = ac::hooks::wire::schemacache::SchemaWrite::Skip;
    if (!ac::hooks::wire::schemacache::StoreTranscribed(appId, bytes, StatsDir(),
                                                        ac::backup::io::BackupDirForApp(appId), in.version,
                                                        static_cast<std::uint32_t>(count), &applied)) {
        AC_LOG_DEBUG(kModule, "App %u: the transcribed schema could not be written.", appId);
        return;
    }
    if (applied != ac::hooks::wire::schemacache::SchemaWrite::Write)
        return; // already current, or the file is Steam's and not ours to touch
    S().schemaFilesWritten.fetch_add(1, std::memory_order_relaxed);
    ac::status::Write();
    AC_LOG_INFO(kModule,
                "App %u: achievement schema %s in appcache\\stats (%zu achievement(s), v%d, %zu byte): "
                "the overlay reads it at game start.",
                appId, existed ? "rewritten" : "transcribed", count, in.version, bytes.size());
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

// --- Catalog prefetch -------------------------------------------------------
//
// The client fixes an app's schema before its first GetUserStats request of the
// session and does not re-read the file, so a catalog transcribed from a later
// page open only helps the next start. Asking the CM for it at startup means the
// file is in place before the client looks, and a session that starts after a
// full wipe (cache and backup) can still unlock.
//
// Each answer is a normal GetGameAchievements reply: OnRecvSchema captures it and
// PublishSchemaFile writes cache, backup and marker. The client never asked for
// these job ids and ignores the replies.

constexpr auto kPrefetchSpacing = std::chrono::milliseconds(150);
constexpr auto kPrefetchSlice = std::chrono::milliseconds(250);
// The send path composes from a captured frame and the first seconds of a
// session have none: wait for it instead of losing the whole batch.
constexpr int kPrefetchReadySlices = 240; // 60 s at kPrefetchSlice
// A request composed while the session is still logging on is never answered;
// one retry pass covers that window.
constexpr auto kPrefetchRetryGap = std::chrono::seconds(8);

// One id range for the calls this module composes itself. The client's job ids
// come from CJobMgr; the high seed keeps the two apart.
std::atomic<std::uint64_t> g_prefetchJobSeed{0xC000000000000000ull};

// <steam>\aethercore\achievement_prefetch.txt: the apps whose reply listed no
// achievement. A hint file, not state the DLL depends on.
std::string PrefetchPath() {
    return g_state.aetherCoreDir.empty() ? std::string() : g_state.aetherCoreDir + "\\achievement_prefetch.txt";
}

prefetch::Markers LoadPrefetchMarkers() {
    const std::string path = PrefetchPath();
    if (path.empty())
        return {};
    std::ifstream file(winfile::ToPath(path), std::ios::binary);
    if (!file.is_open())
        return {};
    const std::string text((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    return prefetch::ParseMarkers(text);
}

void SavePrefetchMarkers(const prefetch::Markers& markers) {
    const std::string path = PrefetchPath();
    if (path.empty())
        return;
    winfile::WriteTextAtomically(path, prefetch::EncodeMarkers(markers));
}

// The schema is written under the language the client asked for plus English.
// A prefetch has no request to borrow the language from, so read the same
// setting the client does; English is the safe fallback.
std::string SteamLanguage() {
    wchar_t buffer[64]{};
    DWORD size = static_cast<DWORD>(sizeof(buffer));
    if (RegGetValueW(HKEY_CURRENT_USER, L"Software\\Valve\\Steam", L"Language", RRF_RT_REG_SZ, nullptr, buffer,
                     &size) != ERROR_SUCCESS)
        return "english";
    const std::wstring text(buffer);
    const std::string narrow = text.empty() ? std::string() : ac::strings::Narrow(text);
    return narrow.empty() ? "english" : narrow;
}

// The CM listed no achievement for this app: remember it so the next start does
// not repeat the question. When a schema file exists it is the stronger proof,
// and no marker is needed.
void NoteEmptyAnswer(std::uint32_t appId) {
    prefetch::Markers markers = LoadPrefetchMarkers();
    markers[appId] = static_cast<std::uint64_t>(std::time(nullptr));
    SavePrefetchMarkers(markers);
}

// Sleeps in slices so the worker stops promptly on shutdown.
bool WaitPrefetch(std::atomic<bool>& stop, std::chrono::milliseconds howLong) {
    for (auto waited = std::chrono::milliseconds(0); waited < howLong; waited += kPrefetchSlice) {
        if (stop)
            return false;
        std::this_thread::sleep_for(std::min(kPrefetchSlice, howLong - waited));
    }
    return !stop;
}

bool SendPrefetch(std::uint32_t appId, const std::string& language) {
    CPlayer_GetGameAchievements_Request request;
    request.set_appid(appId);
    request.set_language(language);
    std::string body;
    body.resize(static_cast<std::size_t>(request.ByteSizeLong()));
    if (!request.SerializeToArray(body.data(), static_cast<int>(body.size())))
        return false;

    Pending pending;
    pending.userCall = false;
    pending.prefetch = true;
    pending.appId = appId;
    pending.language = language;
    const std::uint64_t jobId = g_prefetchJobSeed.fetch_add(1, std::memory_order_relaxed);
    RememberPending(jobId, pending);
    if (!ac::hooks::SendClientServiceCall(constants::job_hash::kGetGameAchievementsName, jobId,
                                          reinterpret_cast<const std::uint8_t*>(body.data()),
                                          static_cast<std::uint32_t>(body.size()))) {
        ForgetPending(jobId);
        return false;
    }
    return true;
}

void PrefetchOnWorker(std::atomic<bool>& stop) {
    const std::string statsDir = StatsDir();
    if (statsDir.empty() || g_state.steamInstallPath.empty())
        return;
    const std::string steamApps = g_state.steamInstallPath + "\\steamapps";
    const std::uint64_t now = static_cast<std::uint64_t>(std::time(nullptr));
    const prefetch::Markers markers = LoadPrefetchMarkers();

    std::vector<std::uint32_t> apps;
    std::size_t installedWithoutSchema = 0;
    for (const steam::AppId appId : ac::luadata::LibraryAppIds()) {
        if (winfile::Exists(StatsSchemaPath(appId)))
            continue; // Steam's own download, or an earlier transcription
        // Only installed apps can run in this session; an app installed later
        // gets its catalog at the next start.
        if (!winfile::Exists(steamApps + "\\appmanifest_" + std::to_string(appId) + ".acf"))
            continue;
        ++installedWithoutSchema;
        const auto it = markers.find(appId);
        const bool answerFresh = it != markers.end() && prefetch::IsAnswerFresh(it->second, now);
        if (prefetch::ShouldAsk(false, answerFresh))
            apps.push_back(appId);
    }
    if (apps.empty()) {
        AC_LOG_DEBUG(kModule, "Prefetch: nothing to ask (%zu installed app(s) without a schema).",
                     installedWithoutSchema);
        return;
    }

    bool ready = false;
    for (int slice = 0; slice < kPrefetchReadySlices && !stop; ++slice) {
        if (ac::hooks::CanSendServiceCall()) {
            ready = true;
            break;
        }
        std::this_thread::sleep_for(kPrefetchSlice);
    }
    if (!ready) {
        AC_LOG_DEBUG(kModule, "Prefetch: send path never became usable; %zu app(s) stay for the next start.",
                     apps.size());
        return;
    }

    const std::string language = SteamLanguage();
    std::size_t sent = 0;
    for (int pass = 0; pass < 2 && !apps.empty() && !stop; ++pass) {
        if (pass != 0) {
            if (!WaitPrefetch(stop, kPrefetchRetryGap))
                return;
            // Drop what the first pass covered: the retry exists only for
            // requests the CM ignored while the session was still logging on.
            std::erase_if(apps, [](std::uint32_t id) { return winfile::Exists(StatsSchemaPath(id)); });
            if (apps.empty())
                break;
        }
        for (std::size_t i = 0; i < apps.size(); ++i) {
            if (stop)
                return;
            if (!SendPrefetch(apps[i], language))
                continue;
            ++sent;
            if (i + 1 < apps.size() && !WaitPrefetch(stop, kPrefetchSpacing))
                return;
        }
    }
    if (sent == 0) {
        AC_LOG_DEBUG(kModule, "Prefetch: no request went out (%zu app(s) without a schema).", apps.size());
        return;
    }
    S().prefetchRequests.fetch_add(sent, std::memory_order_relaxed);
    ac::status::Write();
    AC_LOG_INFO(kModule, "Prefetch: asked the CM for %zu catalog(s) (%zu installed app(s) had no schema file).", sent,
                installedWithoutSchema);
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

void PrefetchMissingSchemas() {
    if (!Enabled())
        return;
    if (!workers::StartWorker("achievement_prefetch", [](std::atomic<bool>& stop) { PrefetchOnWorker(stop); })) {
        AC_LOG_DEBUG(kModule, "Prefetch: worker refused (registry stopped).");
    }
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
        // For a prefetch request, an empty list is an answer worth remembering:
        // the app has no achievement, and asking again at every start would be
        // noise. The marker expires in a week (an update can add some).
        if (pending.prefetch)
            NoteEmptyAnswer(pending.appId);
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
