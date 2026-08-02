#include "pch.h"
#include "hooks/ipc/IpcReply.h"

#include <cstring>

#include "core/Constants.h"

namespace ac::hooks::ipcreply {

bool CanWrite(steam::CUtlBuffer* pWrite, std::size_t size) {
    if (!pWrite || !pWrite->Base()) return false;
    const auto put = pWrite->TellPut();
    if (put < 0) return false;
    return static_cast<std::size_t>(put) >= size;
}

bool Begin(steam::CUtlBuffer* pWrite) {
    if (!CanWrite(pWrite, 1)) return false;
    pWrite->Base()[0] = constants::kIpcReplyTag;
    return true;
}

bool WriteAt(steam::CUtlBuffer* pWrite, std::size_t offset,
             const void* data, std::size_t bytes) {
    if (!pWrite || !pWrite->Base() || !data) return false;
    if (bytes == 0) return true;
    const auto put = pWrite->TellPut();
    if (put < 0) return false;
    if (offset > static_cast<std::size_t>(put) ||
        bytes > static_cast<std::size_t>(put) - offset) {
        return false;  // would overflow the caller buffer
    }
    std::memcpy(pWrite->Base() + offset, data, bytes);
    return true;
}

bool WriteU32(steam::CUtlBuffer* pWrite, std::size_t offset, std::uint32_t value) {
    return WriteAt(pWrite, offset, &value, sizeof(value));
}

bool WriteU64(steam::CUtlBuffer* pWrite, std::size_t offset, std::uint64_t value) {
    return WriteAt(pWrite, offset, &value, sizeof(value));
}

}  // namespace ac::hooks::ipcreply
