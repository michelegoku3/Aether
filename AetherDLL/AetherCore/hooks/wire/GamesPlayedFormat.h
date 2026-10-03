#pragma once

#include <cstdint>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "core/Settings.h"
#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Pure formatting/parsing helpers for games_played wire frames.
//
// Extracted from GamesPlayedModule (P9): NO protobuf, NO g_state, NO logging
// — every function is total and unit-testable on synthetic data (see tests,
// case "gamesplayed_format").
// ---------------------------------------------------------------------------
namespace ac::hooks::GamesPlayedFormat {

// Low-24 appid bits of a CGameID.
steam::AppId AppIdFromGameId(std::uint64_t gameId);

// Human-friendly exe stem: path strip, extension strip, Unreal
// "-Win64-Shipping"-style suffixes and trailing "-steam" removed.
std::string ImageStem(std::string_view imageName);

// A display name is usable when non-empty and not (a case/space-fold of)
// "spacewar" — the bare mask name leaks nothing about the real game.
bool NameIsUsable(const std::string& name);

// "<name>" + hidden appid carrier. Invisible channel by default
// (U+200B mark + 6 VS nibbles); ASCII "<name> | <appid>" when the setting
// says so (legacy receivers).
std::string WithAppIdSuffix(const std::string& name, steam::AppId appId,
                            const Settings& settings);

// Preferred appid carrier: magic+version+LE appid packed into
// game_data_blob (never rendered by client UIs).
std::string MakeAppIdBlob(steam::AppId appId);

// Walks Steam binary KV1 (0x00 struct, 0x01 string, 0x08 end). Stops cleanly
// on truncation/malformed bytes, keeping everything parsed so far.
void ExtractStringKVs(const std::uint8_t* data, std::uint32_t size,
                      std::vector<std::pair<std::string, std::string>>& out);

}  // namespace ac::hooks::GamesPlayedFormat
