#include "cases.h"

#include <cstdint>
#include <string>
#include <vector>

#include "hooks/wire/AchievementUiPolicy.h"
#include "steam_messages.pb.h"

// Golden blobs captured from the client (build 1793740892) while Steam
// showed a game page's Achievements tab. They pin the field numbers and wire
// types of the two service messages Aether rebuilds: a proto edit that moves a
// field changes what the client sees, so the test compares against Valve's own
// bytes instead of round-tripping Aether's encoder.
//
//   kSchemaReply: Player.GetGameAchievements#1 response for app 730 (owned CS2):
//                 one achievement, schema version 247, schema hash, group block.
//   kUserReply:   Player.GetUserAchievements#1 response for the same app: one
//                 unlocked achievement, the echoed version/hash and the group
//                 summary.
namespace {

constexpr std::uint8_t kSchemaReply[] = {
    0x0A, 0xA7, 0x01, 0x0A, 0x08, 0x50, 0x4C, 0x41, 0x59, 0x5F, 0x43, 0x53, 0x32, 0x12, 0x0F, 0x41, 0x20, 0x4E, 0x65,
    0x77, 0x20, 0x42, 0x65, 0x67, 0x69, 0x6E, 0x6E, 0x69, 0x6E, 0x67, 0x1A, 0x1C, 0x54, 0x68, 0x69, 0x73, 0x20, 0x69,
    0x73, 0x20, 0x43, 0x6F, 0x75, 0x6E, 0x74, 0x65, 0x72, 0x2D, 0x53, 0x74, 0x72, 0x69, 0x6B, 0x65, 0x2C, 0x20, 0x74,
    0x6F, 0x6F, 0x2E, 0x22, 0x2C, 0x66, 0x37, 0x35, 0x64, 0x64, 0x30, 0x34, 0x66, 0x61, 0x31, 0x32, 0x34, 0x34, 0x35,
    0x61, 0x38, 0x65, 0x63, 0x34, 0x33, 0x62, 0x65, 0x36, 0x35, 0x66, 0x61, 0x31, 0x36, 0x66, 0x66, 0x31, 0x62, 0x38,
    0x64, 0x32, 0x62, 0x66, 0x38, 0x32, 0x65, 0x2E, 0x6A, 0x70, 0x67, 0x2A, 0x2C, 0x65, 0x37, 0x64, 0x37, 0x39, 0x65,
    0x35, 0x32, 0x30, 0x30, 0x32, 0x63, 0x37, 0x35, 0x66, 0x30, 0x32, 0x37, 0x61, 0x35, 0x31, 0x65, 0x38, 0x65, 0x62,
    0x30, 0x35, 0x61, 0x62, 0x35, 0x35, 0x34, 0x31, 0x36, 0x36, 0x37, 0x38, 0x66, 0x30, 0x39, 0x2E, 0x6A, 0x70, 0x67,
    0x30, 0x01, 0x3A, 0x04, 0x34, 0x31, 0x2E, 0x31, 0x40, 0x80, 0xCC, 0x05, 0x58, 0x00, 0x60, 0x00, 0x68, 0x00, 0x10,
    0xF7, 0x01, 0x1A, 0x12, 0x08, 0x00, 0x12, 0x00, 0x18, 0x00, 0x20, 0x00, 0x28, 0x00, 0x30, 0x00, 0x38, 0x01, 0x40,
    0x01, 0x48, 0x01, 0x20, 0xF5, 0x8D, 0x9A, 0x88, 0x0D,
};

constexpr std::uint8_t kUserReply[] = {
    0x0A, 0x0B, 0x08, 0x80, 0xCC, 0x05, 0x10, 0x01, 0x1D, 0x53, 0x4F, 0x49, 0x6A, 0x10, 0xF7, 0x01, 0x18, 0xF5, 0x8D,
    0x9A, 0x88, 0x0D, 0x22, 0x0D, 0x08, 0x00, 0x10, 0x01, 0x18, 0x01, 0x20, 0x01, 0x4D, 0x53, 0x4F, 0x49, 0x6A,
};

// 91648 = bucket 358 * 256 + bit 0: the internal key the client keys the page by.
constexpr std::uint32_t kKey = 91648;
constexpr std::uint32_t kHash = 3506865909u;       // 0xD10686F5, echoed in both replies
constexpr std::uint32_t kUnlockTime = 1783189331u; // 0x6A494F53

} // namespace

void AchievementServiceWire() {
    // --- the schema reply parses with the field numbers we declared -----------
    {
        CPlayer_GetGameAchievements_Response rsp;
        CHECK(rsp.ParseFromArray(kSchemaReply, sizeof(kSchemaReply)));
        CHECK(rsp.achievements_size() == 1);
        const auto& achievement = rsp.achievements(0);
        CHECK(achievement.internal_name() == "PLAY_CS2");
        CHECK(achievement.name() == "A New Beginning");
        CHECK(achievement.description() == "This is Counter-Strike, too.");
        CHECK(achievement.icon_gray().size() == 44);
        CHECK(achievement.hidden() == 1);
        CHECK(achievement.global_percent() == "41.1");
        CHECK(achievement.internal_key() == kKey);
        CHECK(rsp.schema_version() == 247);
        CHECK(rsp.schema_hash() == kHash);
        // Field 3 is the group list: one entry, group 0 (the counts travel as
        // unknown fields inside it and survive a re-serialization).
        CHECK(rsp.groups_size() == 1);
        CHECK(rsp.groups(0).groupid() == 0);
    }

    // --- the user reply: one entry plus the echoed version/hash --------------
    {
        CPlayer_GetUserAchievements_Response rsp;
        CHECK(rsp.ParseFromArray(kUserReply, sizeof(kUserReply)));
        CHECK(rsp.achievements_size() == 1);
        const auto& entry = rsp.achievements(0);
        CHECK(entry.internal_key() == kKey);
        CHECK(entry.unlocked());
        CHECK(entry.unlock_time() == kUnlockTime);
        CHECK(!entry.has_progress_int());
        CHECK(rsp.schema_version() == 247);
        CHECK(rsp.schema_hash() == kHash);
        CHECK(rsp.groups_size() == 1);
        const auto& group = rsp.groups(0);
        CHECK(group.groupid() == 0);
        CHECK(group.is_achievable()); // field 2 in Valve's reply
        CHECK(group.unlocked() == 1); // counts Valve fills in, a rebuild leaves them out
        CHECK(group.complete() == 1);
        CHECK(group.completed_at() == kUnlockTime);
    }

    // --- a rebuilt reply carries one entry per schema achievement ------------
    // Covers what the glue writes: an entry for every key (locked ones too), the
    // unlock pair only when unlocked, progress_float always, and one achievable
    // group per schema group. Version/hash stay exactly as Valve sent them.
    {
        ac::achieve::Decision decision;
        decision.reply.entries = {{kKey, true, kUnlockTime}, {kKey + 1, false, 0}, {kKey + 2, false, 0}};
        decision.reply.groups = {{0}};

        CPlayer_GetUserAchievements_Response rsp;
        CHECK(rsp.ParseFromArray(kUserReply, sizeof(kUserReply)));
        ac::achieve::FillReply(rsp, decision);

        CHECK(rsp.achievements_size() == 3);
        CHECK(rsp.achievements(0).internal_key() == kKey);
        CHECK(rsp.achievements(0).unlocked());
        CHECK(rsp.achievements(0).unlock_time() == kUnlockTime);
        CHECK(rsp.achievements(1).internal_key() == kKey + 1);
        CHECK(!rsp.achievements(1).unlocked());
        CHECK(!rsp.achievements(1).has_unlock_time());
        for (int i = 0; i < rsp.achievements_size(); ++i)
            CHECK(rsp.achievements(i).has_progress_float()); // the page reads it
        CHECK(rsp.groups_size() == 1);
        CHECK(rsp.groups(0).groupid() == 0);
        CHECK(rsp.groups(0).is_achievable());
        CHECK(!rsp.groups(0).has_unlocked()); // counts are the client's to fill
        CHECK(!rsp.groups(0).has_complete());
        CHECK(!rsp.groups(0).has_completed_at());
        CHECK(rsp.schema_version() == 247);
        CHECK(rsp.schema_hash() == kHash);
    }

    // --- an empty Valve body still produces the full page -------------------
    // The CM sends this for an app the account does not own.
    {
        ac::achieve::Decision decision;
        decision.reply.entries = {{kKey, false, 0}};
        decision.reply.groups = {{0}};

        CPlayer_GetUserAchievements_Response rsp;
        ac::achieve::FillReply(rsp, decision);
        CHECK(rsp.achievements_size() == 1);
        CHECK(rsp.achievements(0).internal_key() == kKey);
        CHECK(!rsp.achievements(0).unlocked());
        CHECK(rsp.achievements(0).has_progress_float());
        CHECK(rsp.groups_size() == 1);
        CHECK(rsp.groups(0).is_achievable());
        CHECK(!rsp.has_schema_version());
        CHECK(!rsp.has_schema_hash());
    }

    // --- the counter reaches the wire as progress_float ---------------------
    // The page draws the achievement's bar from this value; it is the raw stat
    // value, not a fraction.
    {
        ac::achieve::Decision decision;
        decision.reply.entries = {{kKey, false, 0, 3.5f}, {kKey + 1, true, kUnlockTime, 12.0f}};
        decision.reply.groups = {{0}};

        CPlayer_GetUserAchievements_Response rsp;
        CHECK(rsp.ParseFromArray(kUserReply, sizeof(kUserReply)));
        ac::achieve::FillReply(rsp, decision);
        CHECK(rsp.achievements_size() == 2);
        CHECK(rsp.achievements(0).has_progress_float());
        CHECK(rsp.achievements(0).progress_float() == 3.5f);
        CHECK(!rsp.achievements(0).unlocked());
        CHECK(rsp.achievements(1).progress_float() == 12.0f);
        CHECK(rsp.achievements(1).unlocked());
    }

    // --- an unknown field from a future build survives a re-serialization ----
    // The client may add fields we do not know; dropping them would make the
    // reply look like a different schema to the page.
    {
        std::string body(reinterpret_cast<const char*>(kUserReply), sizeof(kUserReply));
        body.push_back(0x50); // field 10, varint
        body.push_back(0x2A); // 42
        CPlayer_GetUserAchievements_Response rsp;
        CHECK(rsp.ParseFromArray(body.data(), static_cast<int>(body.size())));
        CHECK(rsp.achievements_size() == 1);
        std::string serialized;
        CHECK(rsp.SerializeToString(&serialized));
        CPlayer_GetUserAchievements_Response again;
        CHECK(again.ParseFromArray(serialized.data(), static_cast<int>(serialized.size())));
        CHECK(again.achievements_size() == 1);
        CHECK(again.achievements(0).internal_key() == kKey);
        CHECK(again.schema_hash() == kHash);
        CHECK(serialized.size() == body.size()); // the unknown field is still there
    }

    // --- requests: field map for the pair the client sends together ----------
    {
        CPlayer_GetGameAchievements_Request schema;
        schema.set_appid(3618390);
        schema.set_language("english");
        std::string bytes;
        CHECK(schema.SerializeToString(&bytes));
        CPlayer_GetGameAchievements_Request parsed;
        CHECK(parsed.ParseFromArray(bytes.data(), static_cast<int>(bytes.size())));
        CHECK(parsed.appid() == 3618390);
        CHECK(parsed.language() == "english");
        CHECK(!parsed.has_hash_only());
    }
    {
        CPlayer_GetUserAchievements_Request user;
        user.set_steamid(76561198158524802ull);
        user.set_appid(3618390);
        std::string bytes;
        CHECK(user.SerializeToString(&bytes));
        CPlayer_GetUserAchievements_Request parsed;
        CHECK(parsed.ParseFromArray(bytes.data(), static_cast<int>(bytes.size())));
        CHECK(parsed.steamid() == 76561198158524802ull);
        CHECK(parsed.appid() == 3618390);
    }
}
