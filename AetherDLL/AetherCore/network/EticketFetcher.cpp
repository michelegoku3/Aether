#include "pch.h"
#include "network/EticketFetcher.h"

#include <mutex>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "core/AetherCoreState.h"
#include "credentials/CredentialStore.h"
#include "credentials/HexCodec.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "network/RuntimeHttp.h"
#include "utils/JsonStringField.h"

namespace ac::eticketfetch {
namespace {

constexpr const char* kModule = "EticketFetch";

}  // namespace

std::optional<TicketPair> Mint(steam::AppId appId, std::span<const std::uint8_t> nonce) {
    if (appId == 0 || nonce.empty()) return std::nullopt;

    const std::string backendUrl = luadata::EticketUrl();
    if (backendUrl.empty()) return std::nullopt;

    MintKey key{appId, hex::Encode(nonce)};
    {
        std::lock_guard<std::mutex> lock(g_state.eticketFetch.mutex);
        auto it = g_state.eticketFetch.cache.find(key);
        if (it != g_state.eticketFetch.cache.end()) {
            AC_LOG_INFO(kModule, "Cache hit app=%u nonce=%zuB.", appId, nonce.size());
            return it->second;
        }
    }

    const std::string payload =
        "{\"app_id\":\"" + std::to_string(appId) +
        "\",\"nonce\":\"" + key.nonceHex + "\"}";
    const auto resp = http::Post(backendUrl, payload, {"Content-Type: application/json"});
    if (resp.networkError || resp.status != 200) {
        ++g_state.eticketFetch.mintFailureCount;
        AC_LOG_WARN(kModule, "Backend fetch failed app=%u status=%d network=%d.",
                    appId, resp.status, resp.networkError ? 1 : 0);
        return std::nullopt;
    }

    TicketPair pair;
    std::string hexStr;
    if (jsonutil::PullStringField(resp.body, "eticket", hexStr)) {
        if (auto bytes = hex::Decode(hexStr)) pair.eticket = std::move(*bytes);
    }
    if (jsonutil::PullStringField(resp.body, "appticket", hexStr)) {
        if (auto bytes = hex::Decode(hexStr)) pair.ownership = std::move(*bytes);
    }

    if (pair.eticket.empty() && pair.ownership.empty()) {
        ++g_state.eticketFetch.mintFailureCount;
        AC_LOG_WARN(kModule, "Backend response unusable app=%u body_bytes=%zu.",
                    appId, resp.body.size());
        return std::nullopt;
    }

    if (!pair.eticket.empty() && !credential::WriteEncryptedTicket(appId, pair.eticket)) {
        ++g_state.eticketFetch.mintFailureCount;
        AC_LOG_WARN(kModule, "Failed to persist minted ETicket for app=%u.", appId);
        return std::nullopt;
    }
    if (!pair.ownership.empty() && !credential::WriteAppOwnershipTicket(appId, pair.ownership)) {
        AC_LOG_WARN(kModule, "Failed to persist minted AppTicket for app=%u.", appId);
    }

    {
        std::lock_guard<std::mutex> lock(g_state.eticketFetch.mutex);
        g_state.eticketFetch.cache.emplace(std::move(key), pair);
    }

    ++g_state.eticketFetch.mintSuccessCount;
    AC_LOG_INFO(kModule, "Minted tickets app=%u eticket=%zuB ownership=%zuB.",
                appId, pair.eticket.size(), pair.ownership.size());
    return pair;
}

std::size_t CacheCount() {
    std::lock_guard<std::mutex> lock(g_state.eticketFetch.mutex);
    return g_state.eticketFetch.cache.size();
}

}  // namespace ac::eticketfetch
