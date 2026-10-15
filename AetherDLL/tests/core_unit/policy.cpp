#include "cases.h"

#include "propagate/PropagationPolicy.h"

void Policy() {

    using ac::payloadprop::BlockedReason;

    CHECK(BlockedReason(L"C:\\Games\\title.exe", L"C:\\Windows") == nullptr);

    CHECK(BlockedReason(L"C:\\WindowsGames\\title.exe", L"C:\\Windows") == nullptr);

    CHECK(BlockedReason(L"C:/WINDOWS/System32/cmd.exe", L"c:\\windows\\") != nullptr);

    CHECK(BlockedReason(L"C:\\Games\\EasyAntiCheat_EOS.exe", L"C:\\Windows") != nullptr);

    CHECK(BlockedReason(L"C:\\Games\\BEService_x64.exe", L"C:\\Windows") != nullptr);

    CHECK(BlockedReason(L"C:\\Games\\CrashReportClient.exe", L"C:\\Windows") != nullptr);

    CHECK(BlockedReason(L"C:\\Games\\msedgewebview2.exe", L"C:\\Windows") != nullptr);

    CHECK(BlockedReason(L"title.exe", L"C:\\Windows") != nullptr);

    CHECK(BlockedReason(L"", L"C:\\Windows") != nullptr);

    CHECK(BlockedReason(L"C:\\Games\\title.exe", L"") != nullptr);
}
