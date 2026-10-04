#include "pch.h"
#include "network/EticketFetcher.h"

#include <chrono>
#include <condition_variable>
#include <deque>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "credentials/CredentialStore.h"
#include "credentials/HexCodec.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "scripting/LuaData.h"
#include "network/RuntimeHttp.h"
#include "utils/JsonStringField.h"

namespace ac::eticketfetch {
namespace {

constexpr const char* kModule = "EticketFetch";
// La cache è keyed per (app, nonce): i nonce cambiano a ogni richiesta, quindi
// senza cap crescerebbe per tutta la sessione. Oltre il limite si elimina una
// entry arbitraria: il mint è comunque ri-richiedibile, nessuna funzionalità persa.
constexpr std::size_t kMaxCacheEntries = 1024;

// ---------------------------------------------------------------------------
// Background mint worker (A1): the HTTP POST runs here, never on the caller's
// (IPC/wire) thread. Queue + inflight set provide single-flight per key.
// ---------------------------------------------------------------------------
std::mutex s_mutex;
std::condition_variable s_cv;
std::deque<MintKey> s_queue;
std::unordered_set<MintKey, MintKeyHash> s_inflight;
// I-A: il worker gira in workers::StartWorker. Il wait ha comunque un timeout
// (100 ms) perché workers::Shutdown imposta il flag di stop ma NON notifica
// questa cv: senza timeout il join centralizzato deadlockerebbe.
std::atomic<bool> s_started{false};

// The synchronous mint logic (HTTP + parse + persist). Only ever called from
// the worker thread. Returns the minted pair, or nullopt on any failure.
std::optional<TicketPair> MintSync(steam::AppId appId, const std::string& nonceHex) {
    const std::string backendUrl = luadata::EticketUrl();
    if (backendUrl.empty()) return std::nullopt;

    const std::string payload =
        "{\"app_id\":\"" + std::to_string(appId) +
        "\",\"nonce\":\"" + nonceHex + "\"}";
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

    ++g_state.eticketFetch.mintSuccessCount;
    AC_LOG_INFO(kModule, "Minted tickets app=%u eticket=%zuB ownership=%zuB.",
                appId, pair.eticket.size(), pair.ownership.size());
    return pair;
}

void WorkerMain(std::atomic<bool>& stop) {
    for (;;) {
        MintKey key;
        {
            std::unique_lock<std::mutex> lock(s_mutex);
            // wait_for (non wait): lo stop arriva dal flag del registry senza
            // notify su s_cv; il timeout garantisce il risveglio entro 100 ms.
            if (!s_cv.wait_for(lock, std::chrono::milliseconds(100),
                               [&stop] { return stop.load(std::memory_order_relaxed) ||
                                                !s_queue.empty(); })) {
                if (stop.load(std::memory_order_relaxed)) return;
                continue;   // timeout a vuoto: ricontrolla stop/coda
            }
            if (stop.load(std::memory_order_relaxed)) return;
            if (s_queue.empty()) continue;   // sveglia spuria/notifica tardiva
            key = std::move(s_queue.front());
            s_queue.pop_front();
        }

        std::optional<TicketPair> minted = MintSync(key.appId, key.nonceHex);
        if (minted) {
            std::lock_guard<std::mutex> lock(g_state.eticketFetch.mutex);
            g_state.eticketFetch.cache.emplace(key, std::move(*minted));
            if (g_state.eticketFetch.cache.size() > kMaxCacheEntries) {
                g_state.eticketFetch.cache.erase(g_state.eticketFetch.cache.begin());
                AC_LOG_DEBUG(kModule, "Ticket cache over cap (%zu entries); evicted one.",
                             kMaxCacheEntries);
            }
        }

        {
            std::lock_guard<std::mutex> lock(s_mutex);
            s_inflight.erase(key);
        }
    }
}

void EnsureWorkerStarted() {
    bool expected = false;
    if (s_started.compare_exchange_strong(expected, true)) {
        if (!workers::StartWorker("eticket_mint", WorkerMain)) {
            s_started.store(false, std::memory_order_relaxed);
            AC_LOG_ERROR(kModule, "Could not start the eticket mint worker; "
                                  "ticket minting is disabled for this session.");
        }
    }
}

}  // namespace

void MintAsync(steam::AppId appId, std::span<const std::uint8_t> nonce) {
    if (appId == 0 || nonce.empty()) return;

    const std::string backendUrl = luadata::EticketUrl();
    if (backendUrl.empty()) return;

    const MintKey key{appId, hex::Encode(nonce)};

    // Fast path: already minted.
    {
        std::lock_guard<std::mutex> lock(g_state.eticketFetch.mutex);
        if (g_state.eticketFetch.cache.count(key)) return;
    }

    // Single-flight: skip if already queued/in-flight.
    {
        std::lock_guard<std::mutex> lock(s_mutex);
        if (s_inflight.count(key)) return;
        s_inflight.insert(key);
        s_queue.push_back(key);
    }
    s_cv.notify_one();
    EnsureWorkerStarted();
}

std::size_t InflightCount() {
    std::lock_guard<std::mutex> lock(s_mutex);
    return s_inflight.size();
}

std::size_t CacheCount() {
    std::lock_guard<std::mutex> lock(g_state.eticketFetch.mutex);
    return g_state.eticketFetch.cache.size();
}

void Shutdown() {
    // I-A: stop+join del worker sono già avvenuti in workers::Shutdown()
    // (chiamato prima di eticketfetch::Shutdown in dllmain). Qui resta la
    // pulizia dello stato: coda e inflight non devono sopravvivere.
    std::size_t dropped = 0;
    {
        std::lock_guard<std::mutex> lock(s_mutex);
        dropped = s_queue.size();
        s_queue.clear();
        s_inflight.clear();
    }
    s_cv.notify_all();
    AC_LOG_INFO(kModule, "Shutdown: mint worker already joined by workers; dropped %zu queued request(s).",
                dropped);
}

}  // namespace ac::eticketfetch
