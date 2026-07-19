#include "pch.h"
#include "utils/Hasher.h"

#include <bcrypt.h>

#include <array>
#include <cstdio>
#include <memory>
#include <vector>

#include "core/Constants.h"
#include "core/Logger.h"

#pragma comment(lib, "bcrypt.lib")

namespace ac::hasher {
namespace {

constexpr const char* kModule = "Hasher";
constexpr ULONG kSha256DigestBytes = 32;

bool NtOk(NTSTATUS s) { return s >= 0; }

// RAII wrapper for a bcrypt algorithm/hash pair so every early return cleans up.
struct Sha256Context {
    BCRYPT_ALG_HANDLE alg = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;

    ~Sha256Context() {
        if (hash) BCryptDestroyHash(hash);
        if (alg) BCryptCloseAlgorithmProvider(alg, 0);
    }
};

// RAII wrapper for a C FILE*.
struct FileCloser {
    void operator()(std::FILE* f) const { if (f) std::fclose(f); }
};
using FilePtr = std::unique_ptr<std::FILE, FileCloser>;

std::string ToHex(const std::array<std::uint8_t, kSha256DigestBytes>& digest) {
    static constexpr char kHex[] = "0123456789abcdef";
    std::string out;
    out.reserve(kSha256DigestBytes * 2);
    for (std::uint8_t b : digest) {
        out.push_back(kHex[b >> 4]);
        out.push_back(kHex[b & 0x0F]);
    }
    return out;
}

}  // namespace

std::string ComputeFileSha256(const std::string& filePath) {
    std::FILE* raw = nullptr;
    fopen_s(&raw, filePath.c_str(), "rb");
    FilePtr file(raw);
    if (!file) {
        AC_LOG_ERROR(kModule, "Cannot open file: %s", filePath.c_str());
        return {};
    }

    Sha256Context ctx;
    if (!NtOk(BCryptOpenAlgorithmProvider(&ctx.alg, BCRYPT_SHA256_ALGORITHM, nullptr, 0))) {
        AC_LOG_ERROR(kModule, "BCryptOpenAlgorithmProvider failed.");
        return {};
    }
    if (!NtOk(BCryptCreateHash(ctx.alg, &ctx.hash, nullptr, 0, nullptr, 0, 0))) {
        AC_LOG_ERROR(kModule, "BCryptCreateHash failed.");
        return {};
    }

    std::vector<std::uint8_t> buffer(constants::kHashChunkBytes);
    std::size_t total = 0;
    while (true) {
        std::size_t read = std::fread(buffer.data(), 1, buffer.size(), file.get());
        if (read == 0) break;
        if (!NtOk(BCryptHashData(ctx.hash, buffer.data(), static_cast<ULONG>(read), 0))) {
            AC_LOG_ERROR(kModule, "BCryptHashData failed for %s", filePath.c_str());
            return {};
        }
        total += read;
    }

    std::array<std::uint8_t, kSha256DigestBytes> digest{};
    if (!NtOk(BCryptFinishHash(ctx.hash, digest.data(), kSha256DigestBytes, 0))) {
        AC_LOG_ERROR(kModule, "BCryptFinishHash failed.");
        return {};
    }

    AC_LOG_DEBUG(kModule, "Hashed %zu bytes of %s", total, filePath.c_str());
    return ToHex(digest);
}

}  // namespace ac::hasher
