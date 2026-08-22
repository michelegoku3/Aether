#include "pch.h"
#include "hooks/wire/PlaytimeMirror.h"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <fstream>
#include <map>
#include <unordered_map>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/BackupIo.h"
#include "core/SteamTypes.h"

namespace ac::backup::playtime {
namespace {

constexpr const char* kModule = "Wire.Achievement";

    // ---------------------------------------------------------------------------
    // Backup del TEMPO DI GIOCO per account.
    //
    // Steam conserva il playtime in `userdata\<account>\config\localconfig.vdf`
    // (VDF testuale, sezione Software/Valve/Steam/apps): valori in MINUTI
    // ("Playtime", "Playtime2wks", "PlaytimeDisconnected"), "LastPlayed" unix e
    // i timestamp di sessione in "autocloud" {lastlaunch,lastexit}. Per i giochi
    // gestiti da Aether (non posseduti) e per i non-Steam questi dati esistono
    // SOLO lato client: stessa fragilità degli achievement, stesso rimedio.
    //
    // Output: <AetherData>\backup\playtime\UserPlaytime_<account>.json
    // (un file per account, tutti i giochi con playtime registrato; merge con
    // valore massimo per chiave: il backup non può regredire).
    // ---------------------------------------------------------------------------
    struct PlaytimeEntry {
        std::uint32_t appId = 0;
        std::uint32_t playtimeMin = 0;
        std::uint32_t playtime2wksMin = 0;
        std::uint32_t playtimeDisconnectedMin = 0;
        std::uint32_t lastPlayed = 0;
        std::uint32_t lastLaunch = 0;
        std::uint32_t lastExit = 0;
    };

    // Esito del parsing di una riga VDF.
    enum class VdfLine { None, SectionKey, KeyValue };

    // Parsing di una riga: "chiave" "valore" -> KeyValue; "chiave" da sola ->
    // SectionKey (nei VDF di Steam la graffa di apertura è sulla riga SUCCESSIVA);
    // qualsiasi altro contenuto -> None.
    VdfLine ParseVdfLine(const std::string& line, std::string& key, std::string& value) {
        std::size_t i = 0;
        const std::size_t n = line.size();
        auto skipWs = [&] { while (i < n && (line[i] == ' ' || line[i] == '\t')) ++i; };
        auto readQuoted = [&](std::string& out) {
            skipWs();
            if (i >= n || line[i] != '"') return false;
            ++i;
            out.clear();
            while (i < n && line[i] != '"') out += line[i++];
            ++i;
            return true;
            };
        if (!readQuoted(key)) return VdfLine::None;
        skipWs();
        if (i >= n) return VdfLine::SectionKey;          // "chiave" (graffa dopo)
        if (line[i] == '{' || line[i] == '}') return VdfLine::None;
        if (!readQuoted(value)) return VdfLine::None;
        return VdfLine::KeyValue;
    }

    std::string AsciiLower(std::string s) {
        for (char& c : s) {
            if (c >= 'A' && c <= 'Z') c = static_cast<char>(c - 'A' + 'a');
        }
        return s;
    }

    std::unordered_map<std::uint32_t, PlaytimeEntry> ParseLocalConfigPlaytime(const std::string& path) {
        std::unordered_map<std::uint32_t, PlaytimeEntry> out;
        std::ifstream ifs(path);
        if (!ifs.is_open()) return out;

        std::vector<std::string> stack;   // percorso di sezioni corrente
        std::string pendingSection;
        std::string line;
        while (std::getline(ifs, line)) {
            // Rimuovi commenti/CR
            if (!line.empty() && line.back() == '\r') line.pop_back();
            // Trova la prima parentesi graffa fuori da virgolette
            std::size_t brace = std::string::npos;
            bool inQuote = false;
            for (std::size_t i = 0; i < line.size(); ++i) {
                if (line[i] == '"') inQuote = !inQuote;
                else if (!inQuote && (line[i] == '{' || line[i] == '}')) { brace = i; break; }
            }

            if (brace != std::string::npos && line[brace] == '{') {
                // Graffa di apertura: la sezione è la chiave in sospeso (riga
                // precedente) oppure unnamed.
                stack.push_back(pendingSection);
                pendingSection.clear();
                continue;
            }
            if (brace != std::string::npos && line[brace] == '}') {
                if (!stack.empty()) stack.pop_back();
                continue;
            }

            std::string key, value;
            const VdfLine kind = ParseVdfLine(line, key, value);
            if (kind == VdfLine::SectionKey) {
                pendingSection = key;   // attende la graffa sulla riga successiva
                continue;
            }
            if (kind != VdfLine::KeyValue) continue;

            // Cerca .../Apps/<appid>[/<sub>] nello stack (nei localconfig reali la
            // sezione è "Apps" con la maiuscola: confronto case-insensitive).
            int appsIdx = -1;
            for (std::size_t i = 0; i + 1 < stack.size(); ++i) {
                if (AsciiLower(stack[i]) == "apps") { appsIdx = static_cast<int>(i); break; }
            }
            if (appsIdx < 0) continue;
            if (stack.size() < static_cast<std::size_t>(appsIdx) + 2) continue;
            const std::string& appIdStr = stack[appsIdx + 1];
            char* end = nullptr;
            unsigned long appId = std::strtoul(appIdStr.c_str(), &end, 10);
            if (!end || *end != '\0') continue;   // id non numerico (es. non-Steam hash): skip
            const bool inAutocloud = stack.size() == static_cast<std::size_t>(appsIdx) + 3 &&
                stack[appsIdx + 2] == "autocloud";

            PlaytimeEntry& e = out[static_cast<std::uint32_t>(appId)];
            e.appId = static_cast<std::uint32_t>(appId);
            unsigned long v = std::strtoul(value.c_str(), nullptr, 10);
            if (inAutocloud) {
                if (key == "lastlaunch") e.lastLaunch = static_cast<std::uint32_t>(v);
                else if (key == "lastexit") e.lastExit = static_cast<std::uint32_t>(v);
            }
            else {
                if (key == "Playtime") e.playtimeMin = static_cast<std::uint32_t>(v);
                else if (key == "Playtime2wks") e.playtime2wksMin = static_cast<std::uint32_t>(v);
                else if (key == "PlaytimeDisconnected") e.playtimeDisconnectedMin = static_cast<std::uint32_t>(v);
                else if (key == "LastPlayed") e.lastPlayed = static_cast<std::uint32_t>(v);
            }
        }
        return out;
    }

    // Merge monotono: per ogni campo tieni il massimo tra backup esistente e nuovo.
    std::unordered_map<std::uint32_t, PlaytimeEntry> LoadPlaytimeSnapshot(const std::string& path) {
        std::unordered_map<std::uint32_t, PlaytimeEntry> out;
        std::ifstream ifs(path);
        if (!ifs.is_open()) return out;
        std::string line;
        while (std::getline(ifs, line)) {
            const std::size_t p = line.find("{\"appid\":");
            if (p == std::string::npos) continue;
            PlaytimeEntry e{};
            if (std::sscanf(line.c_str() + p,
                "{\"appid\": %u, \"playtime_min\": %u, \"playtime_2wks_min\": %u, "
                "\"playtime_disconnected_min\": %u, \"last_played\": %u, "
                "\"last_launch\": %u, \"last_exit\": %u}",
                &e.appId, &e.playtimeMin, &e.playtime2wksMin,
                &e.playtimeDisconnectedMin, &e.lastPlayed, &e.lastLaunch,
                &e.lastExit) == 7) {
                out[e.appId] = e;
            }
        }
        return out;
    }

    void BackupPlaytimeForAccountDir(const std::string& accountDir, std::uint32_t accountId) {
        const std::string vdf = accountDir + "\\config\\localconfig.vdf";
        auto fresh = ParseLocalConfigPlaytime(vdf);
        if (fresh.empty()) return;

        const std::string deskData = io::CachedDeskDataDir();
        if (deskData.empty()) return;
        const std::string dir = deskData + "\\backup\\playtime";
        CreateDirectoryA(dir.c_str(), nullptr);
        const std::string path = dir + "\\UserPlaytime_" + std::to_string(accountId) + ".json";

        auto merged = LoadPlaytimeSnapshot(path);
        for (auto& [appId, e] : fresh) {
            auto it = merged.find(appId);
            if (it == merged.end()) { merged.emplace(appId, e); continue; }
            auto& m = it->second;
            m.playtimeMin = std::max(m.playtimeMin, e.playtimeMin);
            m.playtime2wksMin = std::max(m.playtime2wksMin, e.playtime2wksMin);
            m.playtimeDisconnectedMin = std::max(m.playtimeDisconnectedMin, e.playtimeDisconnectedMin);
            m.lastPlayed = std::max(m.lastPlayed, e.lastPlayed);
            m.lastLaunch = std::max(m.lastLaunch, e.lastLaunch);
            m.lastExit = std::max(m.lastExit, e.lastExit);
        }

        std::vector<const PlaytimeEntry*> sorted;
        sorted.reserve(merged.size());
        for (const auto& [appId, e] : merged) sorted.push_back(&e);
        std::sort(sorted.begin(), sorted.end(),
            [](const PlaytimeEntry* a, const PlaytimeEntry* b) { return a->appId < b->appId; });

        const std::string tmp = path + ".tmp";
        {
            std::ofstream out(tmp, std::ios::trunc);
            if (!out.is_open()) return;
            out << "{\n";
            out << "  \"_format\": \"aether-playtime-mirror-v1\",\n";
            out << "  \"_note\": \"minutes; source userdata/<account>/config/localconfig.vdf\",\n";
            out << "  \"account_id\": " << accountId << ",\n";
            out << "  \"steamid64\": " << steam::MakeSteamId64(accountId) << ",\n";
            out << "  \"updated\": \"" << io::FormatWallClockNow() << "\",\n";
            out << "  \"updated_unix\": " << static_cast<long long>(std::time(nullptr)) << ",\n";
            out << "  \"apps\": [\n";
            for (std::size_t i = 0; i < sorted.size(); ++i) {
                const PlaytimeEntry& e = *sorted[i];
                out << "    {\"appid\": " << e.appId
                    << ", \"playtime_min\": " << e.playtimeMin
                    << ", \"playtime_2wks_min\": " << e.playtime2wksMin
                    << ", \"playtime_disconnected_min\": " << e.playtimeDisconnectedMin
                    << ", \"last_played\": " << e.lastPlayed
                    << ", \"last_launch\": " << e.lastLaunch
                    << ", \"last_exit\": " << e.lastExit << "}"
                    << (i + 1 < sorted.size() ? "," : "") << "\n";
            }
            out << "  ]\n}\n";
            out.flush();
        }
        if (io::AtomicReplace(tmp, path)) {
            AC_LOG_INFO(kModule, "Backup: playtime for account %u -> %zu app(s) in %s.",
                accountId, merged.size(), path.c_str());
        }
        else {
            AC_LOG_WARN(kModule, "Backup: cannot replace playtime snapshot %s.", path.c_str());
        }
    }

    void ProcessPlaytimeBackup() {
        const std::string userdata = g_state.steamInstallPath + "\\userdata";
        const std::string pattern = userdata + "\\*";
        WIN32_FIND_DATAA fd{};
        HANDLE find = FindFirstFileA(pattern.c_str(), &fd);
        if (find == INVALID_HANDLE_VALUE) return;
        do {
            if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) || fd.cFileName[0] == '.') continue;
            char* end = nullptr;
            const unsigned long account = std::strtoul(fd.cFileName, &end, 10);
            if (!end || *end != '\0' || account == 0) continue;
            BackupPlaytimeForAccountDir(userdata + "\\" + fd.cFileName,
                static_cast<std::uint32_t>(account));
        } while (FindNextFileA(find, &fd));
        FindClose(find);
    }

}  // namespace

void RefreshAllAccounts() {
    ProcessPlaytimeBackup();
}

}  // namespace ac::backup::playtime
