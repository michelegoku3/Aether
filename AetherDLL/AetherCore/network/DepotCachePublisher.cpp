#include "pch.h"
#include "network/DepotCachePublisher.h"

#include "core/Logger.h"

namespace ac::manifestfetch::publisher {
namespace {

constexpr const char* kModule = "ManifestFetch";

} // namespace

bool IsUsableManifestFile(const std::filesystem::path& path) {
    namespace fs = std::filesystem;
    std::error_code ec;
    if (!fs::is_regular_file(path, ec) || ec)
        return false;
    const auto size = fs::file_size(path, ec);
    return !ec && size > 0;
}

std::optional<std::filesystem::path> Publish(const std::filesystem::path& source,
                                             const std::filesystem::path& steamRoot, const std::string& filename) {
    namespace fs = std::filesystem;
    std::error_code ec;
    fs::path destination = steamRoot / "depotcache" / filename;
    fs::create_directories(destination.parent_path(), ec);
    if (ec)
        return std::nullopt;
    fs::copy_file(source, destination, fs::copy_options::overwrite_existing, ec);
    if (ec || !IsUsableManifestFile(destination)) {
        AC_LOG_WARN(kModule, "Local backup manifest found but could not be published to %s (%s).",
                    destination.string().c_str(), ec.message().c_str());
        return std::nullopt;
    }
    return destination;
}

} // namespace ac::manifestfetch::publisher
