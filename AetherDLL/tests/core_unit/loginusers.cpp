#include "cases.h"

#include "credentials/LoginUsers.h"

void LoginUsersTest() {
    using ac::steamid::SelectLoginAccount;

    // Real shape: `users` -> "<steamid64>" { ..., "AutoLogin" "1", "timestamp" ... }.
    // The account Steam auto-signs in must win even when another account was
    // used more recently.
    {
        const std::string content = "\"users\"\n"
                                    "{\n"
                                    "\t\"76561199841844794\"\n"
                                    "\t{\n"
                                    "\t\t\"AccountName\"\t\t\"minaf567890\"\n"
                                    "\t\t\"RememberPassword\"\t\t\"1\"\n"
                                    "\t\t\"AutoLogin\"\t\t\"1\"\n"
                                    "\t\t\"timestamp\"\t\t\"1792881142\"\n"
                                    "\t}\n"
                                    "\t\"76561199876393402\"\n"
                                    "\t{\n"
                                    "\t\t\"AccountName\"\t\t\"robspont15\"\n"
                                    "\t\t\"RememberPassword\"\t\t\"1\"\n"
                                    "\t\t\"AutoLogin\"\t\t\"0\"\n"
                                    "\t\t\"timestamp\"\t\t\"1792900000\"\n"
                                    "\t}\n"
                                    "}\n";
        CHECK_EQ(SelectLoginAccount(content), 76561199841844794ull);
    }

    // Without an AutoLogin entry, the last account used (newest timestamp) wins.
    {
        const std::string content = "\"users\"\n"
                                    "{\n"
                                    "\t\"76561199841844794\"\n"
                                    "\t{\n"
                                    "\t\t\"AutoLogin\"\t\t\"0\"\n"
                                    "\t\t\"timestamp\"\t\t\"1792000000\"\n"
                                    "\t}\n"
                                    "\t\"76561199876393402\"\n"
                                    "\t{\n"
                                    "\t\t\"AutoLogin\"\t\t\"0\"\n"
                                    "\t\t\"timestamp\"\t\t\"1792500000\"\n"
                                    "\t}\n"
                                    "}\n";
        CHECK_EQ(SelectLoginAccount(content), 76561199876393402ull);
    }

    // CRLF and arbitrary whitespace are Steam's normal on-disk shape.
    {
        const std::string content = "\"users\"\r\n"
                                    "{\r\n"
                                    "\t\"76561199841844794\"\r\n"
                                    "\t{\r\n"
                                    "\t\t\"AutoLogin\"\t\t\"1\"\r\n"
                                    "\t\t\"timestamp\"\t\t\"1\"\r\n"
                                    "\t}\r\n"
                                    "}\r\n";
        CHECK_EQ(SelectLoginAccount(content), 76561199841844794ull);
    }

    // `AutoLogin "0"` never makes an entry the winner by itself.
    {
        const std::string content = "\"users\"\n"
                                    "{\n"
                                    "\t\"76561199841844794\"\n"
                                    "\t{\n"
                                    "\t\t\"AutoLogin\"\t\t\"0\"\n"
                                    "\t\t\"timestamp\"\t\t\"10\"\n"
                                    "\t}\n"
                                    "}\n";
        CHECK_EQ(SelectLoginAccount(content), 76561199841844794ull);
    }

    // A timestamp beats the other entry only when both are non-AutoLogin.
    {
        const std::string content = "\"users\"\n"
                                    "{\n"
                                    "\t\"76561199841844794\"\n"
                                    "\t{\n"
                                    "\t\t\"AutoLogin\"\t\t\"1\"\n"
                                    "\t\t\"timestamp\"\t\t\"10\"\n"
                                    "\t}\n"
                                    "\t\"76561199876393402\"\n"
                                    "\t{\n"
                                    "\t\t\"AutoLogin\"\t\t\"0\"\n"
                                    "\t\t\"timestamp\"\t\t\"999999999\"\n"
                                    "\t}\n"
                                    "}\n";
        CHECK_EQ(SelectLoginAccount(content), 76561199841844794ull);
    }

    // Empty, malformed and non-user VDFs never invent an account.
    CHECK_EQ(SelectLoginAccount(""), 0ull);
    CHECK_EQ(SelectLoginAccount("\"users\"\n{\n}\n"), 0ull);
    CHECK_EQ(SelectLoginAccount("\"not-users\"\n{\n\t\"76561199841844794\"\n\t{\n"
                                "\t\t\"AutoLogin\"\t\t\"1\"\n\t}\n}\n"),
             0ull);
    CHECK_EQ(SelectLoginAccount("\"users\"\n{\n\t\"not-a-steamid\"\n\t{\n"
                                "\t\t\"AutoLogin\"\t\t\"1\"\n\t}\n}\n"),
             0ull);
    CHECK_EQ(SelectLoginAccount("\"users\"\n{\n\t\"76561199841844794\"\n\t{\n"
                                "\t\t\"AutoLogin\"\t\t\"1\"\n"),
             0ull);
}
