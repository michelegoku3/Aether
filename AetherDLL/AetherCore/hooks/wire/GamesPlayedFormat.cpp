#include "pch.h"
#include "hooks/wire/GamesPlayedFormat.h"

#include <cctype>
#include <cstring>

#include "core/Constants.h"

namespace ac::hooks::GamesPlayedFormat {

steam::AppId AppIdFromGameId(std::uint64_t gameId) {
    return static_cast<steam::AppId>(gameId & constants::kGameIdAppIdMask);
}

std::string ImageStem(std::string_view imageName) {
    const auto slash = imageName.find_last_of("\\/");
    std::string file = (slash == std::string_view::npos)
                           ? std::string(imageName)
                           : std::string(imageName.substr(slash + 1));
    const auto dot = file.find_last_of('.');
    if (dot != std::string::npos) file.resize(dot);
    // Unreal: Bodycam-Win64-Shipping -> Bodycam
    static constexpr const char* kShipping[] = {
        "-Win64-Shipping", "-Win32-Shipping", "-Win64-Test", "-Win32-Test",
    };
    for (const char* suf : kShipping) {
        const std::size_t n = std::strlen(suf);
        if (file.size() > n) {
            const std::string tail = file.substr(file.size() - n);
            std::string fold = tail;
            for (char& c : fold) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
            std::string want = suf;
            for (char& c : want) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
            if (fold == want) {
                file.resize(file.size() - n);
                break;
            }
        }
    }
    // ReadyOrNotSteam-Win64-Shipping -> ReadyOrNot
    if (file.size() > 5) {
        std::string tail = file.substr(file.size() - 5);
        for (char& c : tail) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
        if (tail == "steam") file.resize(file.size() - 5);
    }
    return file;
}

bool NameIsUsable(const std::string& name) {
    if (name.empty()) return false;
    std::string fold;
    fold.reserve(name.size());
    for (unsigned char c : name) {
        if (std::isspace(c)) continue;
        fold.push_back(static_cast<char>(std::tolower(c)));
    }
    return fold != "spacewar";
}

std::string WithAppIdSuffix(const std::string& name, steam::AppId appId,
                            const Settings& settings) {
    if (!settings.presenceSuffixInvisible) {
        return name + constants::kExtraInfoAppIdSep + std::to_string(appId);
    }
    std::string out = name + constants::kExtraInfoInvisibleMark;
    constexpr std::size_t digits = constants::kExtraInfoInvisibleDigits;
    for (std::size_t i = 0; i < digits; ++i) {
        const int shift = static_cast<int>((digits - 1 - i) * 4);
        const std::uint8_t nib = static_cast<std::uint8_t>((appId >> shift) & 0xF);
        out += "\xEE\xB8";
        out.push_back(static_cast<char>(0x80 | nib));
    }
    return out;
}

std::string MakeAppIdBlob(steam::AppId appId) {
    std::string b(constants::kAppIdBlobMagic, 4);
    b.push_back(static_cast<char>(constants::kAppIdBlobVersion));
    b.push_back(static_cast<char>(appId & 0xFF));
    b.push_back(static_cast<char>((appId >> 8) & 0xFF));
    b.push_back(static_cast<char>((appId >> 16) & 0xFF));
    b.push_back(static_cast<char>((appId >> 24) & 0xFF));
    return b;
}

void ExtractStringKVs(const std::uint8_t* data, std::uint32_t size,
                      std::vector<std::pair<std::string, std::string>>& out) {
    std::uint32_t pos = 0;
    int depth = 0;
    auto readCStr = [&](std::string& s) -> bool {
        const std::uint32_t start = pos;
        while (pos < size && data[pos] != 0) ++pos;
        if (pos >= size) return false;
        s.assign(reinterpret_cast<const char*>(data + start), pos - start);
        ++pos;
        return true;
    };
    while (pos < size) {
        const std::uint8_t type = data[pos++];
        if (type == 0x08) {
            if (depth > 0) {
                --depth;
                continue;
            }
            break;
        }
        if (type == 0x00) {
            std::string ignored;
            if (!readCStr(ignored)) return;
            ++depth;
        } else if (type == 0x01) {
            std::string key, value;
            if (!readCStr(key) || !readCStr(value)) return;
            out.emplace_back(std::move(key), std::move(value));
        } else {
            return;
        }
    }
}

}  // namespace ac::hooks::GamesPlayedFormat
