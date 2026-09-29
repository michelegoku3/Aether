// Reporting hooks for the test binary.
//
// The ABI guards split *policy* (inline in the headers, exercised by the
// tests) from *reporting* (a log line, a diagnostic record). A test
// executable has no logger and no AetherCoreState, so it supplies the
// reporting side here and nothing else: what the tests check is the real
// decision code, not a re-implementation of it.
//
// If this file ever has to grow logic, that logic is in the wrong place —
// move it into the header where the tests can see it.
#include "core/StructGuard.h"

namespace ac::abi::guard {

void ReportRejection(const char*, Reason) {}
void ReportTableMismatch(const char*, std::uint32_t, std::size_t, const std::string&) {}
void ReportTableAgrees(std::size_t, const std::string&) {}

}  // namespace ac::abi::guard
