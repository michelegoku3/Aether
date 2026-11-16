#include "pch.h"
#include "hooks/wire/FrameOrigin.h"

#include <mutex>

#include "core/Constants.h"

namespace ac::hooks {
namespace {

std::mutex s_originMutex;
void* s_originObj = nullptr;
std::vector<std::uint8_t> s_originHeader;
void* s_serviceObj = nullptr;
std::vector<std::uint8_t> s_serviceHeader;

bool CopyHeader(const WireFrame& f, std::vector<std::uint8_t>& into) {
    if (f.headerLen == 0 || f.headerLen > ac::constants::kWireMaxHeaderBytes)
        return false;
    into.assign(f.header, f.header + f.headerLen);
    return true;
}

} // namespace

void CaptureOrigin(void* obj, const WireFrame& f) {
    std::vector<std::uint8_t> header;
    if (!CopyHeader(f, header))
        return;
    std::lock_guard<std::mutex> lk(s_originMutex);
    s_originObj = obj;
    s_originHeader = std::move(header);
}

void CaptureServiceOrigin(void* obj, const WireFrame& f) {
    std::vector<std::uint8_t> header;
    if (!CopyHeader(f, header))
        return;
    std::lock_guard<std::mutex> lk(s_originMutex);
    s_serviceObj = obj;
    s_serviceHeader = std::move(header);
}

FrameOrigin SnapshotOrigin() {
    std::lock_guard<std::mutex> lk(s_originMutex);
    return FrameOrigin{s_originObj, s_originHeader};
}

FrameOrigin SnapshotServiceOrigin() {
    std::lock_guard<std::mutex> lk(s_originMutex);
    return FrameOrigin{s_serviceObj, s_serviceHeader};
}

} // namespace ac::hooks
