#include "pch.h"
#include "hooks/wire/FrameOrigin.h"

#include <mutex>

#include "core/Constants.h"

namespace ac::hooks {
namespace {

std::mutex s_originMutex;
void* s_originObj = nullptr;
std::vector<std::uint8_t> s_originHeader;

} // namespace

void CaptureOrigin(void* obj, const WireFrame& f) {
    if (f.headerLen == 0 || f.headerLen > ac::constants::kWireMaxHeaderBytes)
        return;
    std::lock_guard<std::mutex> lk(s_originMutex);
    s_originObj = obj;
    s_originHeader.assign(f.header, f.header + f.headerLen);
}

FrameOrigin SnapshotOrigin() {
    std::lock_guard<std::mutex> lk(s_originMutex);
    return FrameOrigin{s_originObj, s_originHeader};
}

} // namespace ac::hooks
