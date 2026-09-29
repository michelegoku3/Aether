#include "pch.h"
#include "core/StructGuard.h"

#include "core/Logger.h"

// Reporting only. Every decision this module makes lives in StructGuard.h so
// the unit tests exercise the real policy instead of a copy of it; what is
// left here is the part that needs a logger, which a test binary does not
// have and should not have to fake.
namespace ac::abi::guard {
namespace {
constexpr const char* kModule = "Abi.StructGuard";
}  // namespace

void ReportRejection(const char* what, Reason reason) {
    // ERROR: this means we refused to write into a Steam struct because it did
    // not look like the struct we compiled against. Either the layout moved
    // (Valve update) or we were handed the wrong pointer. Both need a human.
    AC_LOG_ERROR(kModule, "%s rejected: %s. Write skipped — the feature degrades, Steam does not.",
                 what, ReasonText(reason));
    diag::Record("abi_struct_guard", std::string(what) + ": " + ReasonText(reason));
}

void ReportTableMismatch(const char* field, std::uint32_t published, std::size_t compiled,
                         const std::string& source) {
    AC_LOG_ERROR(kModule,
                 "ABI table (%s): %s is 0x%X in this build but this DLL was compiled "
                 "with 0x%zX. A struct cannot be re-laid-out at runtime, so every "
                 "guarded write is disabled for this session. AetherCore needs a "
                 "rebuild for this build of Steam.",
                 source.c_str(), field, published, compiled);
    diag::Record("abi_table_mismatch", std::string(field) + " published=" +
                                           std::to_string(published) + " compiled=" +
                                           std::to_string(compiled));
}

void ReportTableAgrees(std::size_t matched, const std::string& source) {
    AC_LOG_INFO(kModule, "ABI table (%s): %zu field offset(s) confirm the compiled layout.",
                source.c_str(), matched);
}

}  // namespace ac::abi::guard
