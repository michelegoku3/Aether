#pragma once

#include <string>

// Fetches a per-build pattern TOML over HTTPS from a small mirror chain and
// stores it atomically in the local cache.
namespace ac::downloader {

// subdir is "steamclient" or "steamui"; sha is the DLL's SHA-256; outPath is
// where the body is written on success. Returns true if any mirror succeeded.
// outSource receives "mirror", "github", or "cdn" when provided.
bool Download(const std::string& subdir, const std::string& sha, const std::string& outPath,
              std::string* outSource = nullptr);

}  // namespace ac::downloader
