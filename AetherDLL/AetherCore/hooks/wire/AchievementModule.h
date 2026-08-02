#pragma once

#include <cstdint>
#include "hooks/wire/PacketRouter.h"

namespace ac::hooks::AchievementModule {

// Gestione dei messaggi in uscita (intercettazione richieste statistica per applicare lo Spoofing)
std::int32_t HandleSendGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);
std::int32_t HandleSendClientGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// Gestione dei messaggi in entrata (pulizia dati donatore e iniezione dati locali dell'utente)
std::int32_t HandleRecvGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                                            std::uint8_t* outHdr, std::uint32_t outHdrCap, std::int32_t* outNewHdrLen);
std::int32_t HandleRecvClientGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// Gestione dei salvataggi (Store)
std::int32_t HandleSendStoreUserStats2(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// ---- Donor resolution (A1: async, non-blocking) ---------------------------
// Number of donor app ids currently queued/in-flight on the background worker.
std::size_t PendingDonorResolves();

// Stops and joins the donor worker thread. Safe to call when never started.
void Shutdown();

} // namespace ac::hooks::AchievementModule
