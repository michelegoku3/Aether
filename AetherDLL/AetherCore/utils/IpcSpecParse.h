#pragma once

// ---------------------------------------------------------------------------
// Parser puro del TOML di IPC spec.
//
// Estratto da IpcSpec.cpp::ParseToml: la stessa logica di parsing, ma senza
// toccare g_state né il filesystem, così la suite "ipcspec" di quickwin_tests
// può verificarla in isolamento. IpcSpec.cpp resta l'unico consumatore che
// pubblica il risultato in g_state.ipcSpec.
// ---------------------------------------------------------------------------

#include <cstdint>
#include <string>
#include <unordered_map>

#include "utils/IpcSpec.h"

namespace ac::ipcspec {

// Parsa un documento TOML di IPC spec nelle mappe di output. Ritorna false
// se il documento non è TOML valido o non produce nemmeno un metodo valido
// (stessa semantica di prima: un file vuoto/non parsabile NON deve
// sovrascrivere lo stato esistente — il chiamante pubblica solo su true).
bool ParseSpecToml(const std::string& body,
                   std::unordered_map<std::string, std::uint8_t>& outInterfaceIds,
                   std::unordered_map<std::string, MethodSpec>& outMethods);

}  // namespace ac::ipcspec
