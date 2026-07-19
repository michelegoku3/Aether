#include "pch.h"
#include "credentials/Ticket.h"

#include <algorithm>
#include <cstring>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "credentials/CredentialStore.h"
#include "core/Logger.h"
#include "credentials/SteamId.h"

namespace ac::ticket {
namespace {

constexpr const char* kModule = "Ticket";
constexpr steam::AppId kPreferredForgeSourceAppId = 7;
constexpr std::size_t kForgedTailGap = kAppTicketSignatureSize + sizeof(steam::AppId);

template <typename T>
bool ReadValue(const std::vector<std::uint8_t>& data, std::size_t offset, T& out) {
    if (offset > data.size() || sizeof(T) > data.size() - offset) return false;
    std::memcpy(&out, data.data() + offset, sizeof(T));
    return true;
}

bool IsStatusOk(AppTicketStatus status) {
    return status == AppTicketStatus::OkStandard || status == AppTicketStatus::OkForged;
}

void FillMetadata(OwnershipTicket& ticket, const AppTicketInspection& inspection) {
    ticket.steamIdOffset = kAppTicketSteamIdOffset;
    ticket.signatureSize = kAppTicketSignatureSize;
    if (inspection.status == AppTicketStatus::OkForged) {
        ticket.totalSize = static_cast<std::uint32_t>(
            ticket.data.size() - sizeof(steam::AppId));
        ticket.appIdOffset = inspection.forgedAppIdOffset;
        ticket.signatureOffset = ticket.appIdOffset + sizeof(steam::AppId);
        return;
    }
    ticket.totalSize = static_cast<std::uint32_t>(ticket.data.size());
    ticket.appIdOffset = kAppTicketAppIdOffset;
    ticket.signatureOffset = inspection.signatureOffset;
}

bool FindBestForgeSource(steam::AppId targetAppId, std::uint64_t activeSteamId,
                         steam::AppId& sourceAppId) {
    sourceAppId = 0;
    if (activeSteamId == 0) {
        AC_LOG_DEBUG(kModule, "FindBestForgeSource: activeSteamId is 0, cannot find forge source for app %u.", targetAppId);
        return false;
    }

    auto prefer = credential::ReadAppOwnershipTicket(kPreferredForgeSourceAppId);
    if (InspectAppOwnershipTicket(prefer, kPreferredForgeSourceAppId,
                                  activeSteamId).status == AppTicketStatus::OkStandard) {
        sourceAppId = kPreferredForgeSourceAppId;
        AC_LOG_DEBUG(kModule, "FindBestForgeSource: using preferred AppId %u as forge source.", kPreferredForgeSourceAppId);
        return true;
    }

    std::unordered_set<steam::AppId> candidates;
    {
        std::lock_guard<std::mutex> lock(g_state.configStoreTicketMutex);
        for (const auto& [appId, _] : g_state.configStoreAppTickets) {
            if (appId != targetAppId) candidates.insert(appId);
        }
    }

    HKEY root = nullptr;
    if (RegOpenKeyExA(HKEY_CURRENT_USER, "Software\\Valve\\Steam\\Apps", 0,
                      KEY_ENUMERATE_SUB_KEYS, &root) == ERROR_SUCCESS) {
        for (DWORD index = 0;; ++index) {
            char name[64] = {};
            DWORD nameLen = static_cast<DWORD>(sizeof(name));
            FILETIME ignored{};
            if (RegEnumKeyExA(root, index, name, &nameLen, nullptr, nullptr,
                              nullptr, &ignored) == ERROR_NO_MORE_ITEMS)
                break;
            char* end = nullptr;
            unsigned long parsed = std::strtoul(name, &end, 10);
            if (!end || *end != '\0' || parsed == 0 || parsed > UINT32_MAX) continue;
            if (parsed != targetAppId) candidates.insert(static_cast<steam::AppId>(parsed));
        }
        RegCloseKey(root);
    }

    std::vector<steam::AppId> ordered(candidates.begin(), candidates.end());
    std::sort(ordered.begin(), ordered.end());
    for (steam::AppId candidate : ordered) {
        if (candidate == kPreferredForgeSourceAppId) continue;
        auto source = credential::ReadAppOwnershipTicket(candidate);
        if (InspectAppOwnershipTicket(source, candidate,
                                      activeSteamId).status == AppTicketStatus::OkStandard) {
            sourceAppId = candidate;
            AC_LOG_DEBUG(kModule, "FindBestForgeSource: found candidate AppId %u as forge source for app %u.", candidate, targetAppId);
            return true;
        }
    }

    AC_LOG_DEBUG(kModule, "FindBestForgeSource: no valid ticket found among %zu candidates for target app %u.", candidates.size(), targetAppId);
    return false;
}

}  // namespace

AppTicketInspection InspectAppOwnershipTicket(const std::vector<std::uint8_t>& data,
                                             steam::AppId appId,
                                             std::uint64_t expectedSteamId) {
    AppTicketInspection out{};
    if (data.empty()) {
        out.status = AppTicketStatus::Empty;
        return out;
    }
    if (data.size() <= kForgedTailGap ||
        data.size() < kAppTicketAppIdOffset + sizeof(steam::AppId) ||
        data.size() < kAppTicketSteamIdOffset + sizeof(std::uint64_t) ||
        data.size() < sizeof(std::uint32_t)) {
        out.status = AppTicketStatus::TooSmall;
        AC_LOG_DEBUG(kModule, "InspectTicket app %u: data size %zu too small.", appId, data.size());
        return out;
    }
    if (!ReadValue(data, 0, out.signatureOffset) ||
        out.signatureOffset > data.size() ||
        kAppTicketSignatureSize > data.size() - out.signatureOffset) {
        out.status = AppTicketStatus::TooSmall;
        AC_LOG_DEBUG(kModule, "InspectTicket app %u: signature offset invalid.", appId);
        return out;
    }

    ReadValue(data, kAppTicketSteamIdOffset, out.steamId);
    ReadValue(data, kAppTicketAppIdOffset, out.standardAppId);

    out.forgedAppIdOffset = static_cast<std::uint32_t>(data.size() - kForgedTailGap);
    if (out.forgedAppIdOffset != kAppTicketAppIdOffset)
        ReadValue(data, out.forgedAppIdOffset, out.forgedAppId);
    else
        out.forgedAppId = out.standardAppId;

    if (expectedSteamId != 0 && out.steamId != expectedSteamId) {
        out.status = AppTicketStatus::SteamIdMismatch;
        AC_LOG_DEBUG(kModule, "InspectTicket app %u: SteamID mismatch (ticket=%llu expected=%llu).",
                     appId, static_cast<unsigned long long>(out.steamId),
                     static_cast<unsigned long long>(expectedSteamId));
        return out;
    }
    if (out.standardAppId == appId) {
        out.status = AppTicketStatus::OkStandard;
        return out;
    }
    if (out.forgedAppId == appId) {
        out.status = AppTicketStatus::OkForged;
        return out;
    }
    out.status = AppTicketStatus::AppIdMismatch;
    AC_LOG_DEBUG(kModule, "InspectTicket app %u: AppId mismatch (standard=%u forged=%u target=%u).",
                 appId, out.standardAppId, out.forgedAppId, appId);
    return out;
}

std::vector<std::uint8_t> ForgeAppOwnershipTicket(steam::AppId sourceAppId,
                                                  steam::AppId targetAppId) {
    std::vector<std::uint8_t> source = credential::ReadAppOwnershipTicket(sourceAppId);
    if (source.empty()) {
        AC_LOG_WARN(kModule, "ForgeTicket: source ticket for AppId %u is empty.", sourceAppId);
        return {};
    }

    const std::uint64_t activeSteamId = steamid::GetActiveSteamId64();
    const AppTicketInspection insp =
        InspectAppOwnershipTicket(source, sourceAppId, activeSteamId);
    if (insp.status != AppTicketStatus::OkStandard) {
        AC_LOG_WARN(kModule, "ForgeTicket: source ticket for AppId %u failed inspection.", sourceAppId);
        return {};
    }

    if (source.size() < kAppTicketSignatureSize) {
        AC_LOG_WARN(kModule, "ForgeTicket: source ticket size %zu smaller than signature size.", source.size());
        return {};
    }

    const std::size_t bodyLen = source.size() - kAppTicketSignatureSize;
    std::vector<std::uint8_t> ticket;
    ticket.reserve(source.size() + sizeof(steam::AppId));
    ticket.insert(ticket.end(), source.begin(),
                  source.begin() + static_cast<std::ptrdiff_t>(bodyLen));
    auto* appIdBytes = reinterpret_cast<const std::uint8_t*>(&targetAppId);
    ticket.insert(ticket.end(), appIdBytes, appIdBytes + sizeof(steam::AppId));
    ticket.insert(ticket.end(),
                  source.begin() + static_cast<std::ptrdiff_t>(bodyLen), source.end());
    AC_LOG_DEBUG(kModule, "ForgeTicket: forged ticket for target app %u using source app %u (new size %zu).",
                 targetAppId, sourceAppId, ticket.size());
    return ticket;
}

bool GetAppOwnershipTicket(steam::AppId appId, OwnershipTicket& ticketOut) {
    ticketOut = {};
    const std::uint64_t activeSteamId = steamid::GetActiveSteamId64();

    ticketOut.data = credential::ReadAppOwnershipTicket(appId);
    AppTicketInspection inspection =
        InspectAppOwnershipTicket(ticketOut.data, appId, activeSteamId);
    if (IsStatusOk(inspection.status)) {
        FillMetadata(ticketOut, inspection);
        AC_LOG_DEBUG(kModule, "GetAppOwnershipTicket: valid ticket found for app %u.", appId);
        return true;
    }

    steam::AppId sourceAppId = 0;
    if (!FindBestForgeSource(appId, activeSteamId, sourceAppId)) {
        ++g_state.ticketForgeFailureCount;
        AC_LOG_DEBUG(kModule, "No forge source for app %u.", appId);
        ticketOut = {};
        return false;
    }

    ticketOut.data = ForgeAppOwnershipTicket(sourceAppId, appId);
    inspection = InspectAppOwnershipTicket(ticketOut.data, appId, activeSteamId);
    if (!IsStatusOk(inspection.status)) {
        ++g_state.ticketForgeFailureCount;
        AC_LOG_WARN(kModule, "Forged AppTicket invalid for app %u from source %u.",
                    appId, sourceAppId);
        ticketOut = {};
        return false;
    }

    FillMetadata(ticketOut, inspection);
    ++g_state.ticketForgeSuccessCount;
    AC_LOG_INFO(kModule, "Forged AppTicket for app %u from source %u (%zu bytes).",
                appId, sourceAppId, ticketOut.data.size());
    return true;
}

}  // namespace ac::ticket
