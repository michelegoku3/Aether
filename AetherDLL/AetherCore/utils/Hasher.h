#pragma once

#include <string>

// SHA-256 helper backed by the Windows CNG (bcrypt) API, so there is no
// third-party crypto dependency.
namespace ac::hasher {

// Streams filePath and returns the lowercase hex SHA-256 digest, or an empty
// string on any failure (logged).
std::string ComputeFileSha256(const std::string& filePath);

}  // namespace ac::hasher
