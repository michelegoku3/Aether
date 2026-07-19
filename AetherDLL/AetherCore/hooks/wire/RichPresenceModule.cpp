#include "pch.h"
#include "hooks/wire/RichPresenceModule.h"

#include "hooks/wire/PersonaInject.h"

namespace ac::hooks::RichPresence {

std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    return PersonaInject::OnPersonaStateRecv(frame, out, outCap);
}

}  // namespace ac::hooks::RichPresence
