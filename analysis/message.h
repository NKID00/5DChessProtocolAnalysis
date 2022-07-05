#include <stdint.h>

/* notes:
All data are little-endian.
The field length excludes itself.
Comments of unknown fields are seen values.
Passcode seems to identify not started matches (both public and private).
Other two tokens seems to identify every match and every S2C action message.
All judgments is performed locally thus it is impossible to cheat. */

struct C2SGreet
{
    uint64_t length; /* = 56 */
    int64_t type; /* = 1 */
    int64_t version1; /* = 11 */
    int64_t version2; /* = 16 */
    int64_t unknown1; /* = 0 */
    int64_t unknown2; /* = 0 */
    int64_t unknown3; /* = 0 */
    int64_t unknown4; /* = 0 */
};

struct S2CGreet
{
    uint64_t length; /* = 56 */
    int64_t type; /* = 2 */
    int64_t version; /* unconfirmed, = 1 */
    int64_t unknown1; /* = 0 */
    int64_t unknown2; /* = 0 */
    int64_t unknown3; /* = 0 */
    int64_t unknown4; /* = 0 */
    int64_t unknown5; /* = 0 */
};

struct C2SMatchCreateOrJoin
{
    uint64_t length; /* = 48 */
    int64_t type; /* = 3 */
    int64_t color; /* Join = 0, Random = 1, White = 2, Black = 3 */
    int64_t clock; /* Join = 0, No Clock = 1, Short = 2, Medium = 3, Long = 4 */
    int64_t variant; /* Join = 0, Standard = 1, Random = 34, Turn Zero = 35, ... */
    int64_t visibility; /* Join = 0, Public = 1, Private = 2 */
    int64_t passcode; /* Join = passcode, Create = -1 */
};

struct S2CMatchCreateOrJoinResult
{
    uint64_t length; /* = 64 */
    int64_t type; /* = 4 */
    int64_t result; /* Success = 1, Failed = 0 */
    int64_t reason; /* unconfirmed, Success = 0, Match Not Found = 1 */
    int64_t color; /* Failed = 0, Random = 1, White = 2, Black = 3 */
    int64_t clock; /* Failed = 0, No Clock = 1, Short = 2, Medium = 3, Long = 4 */
    int64_t variant; /* Failed = 0, Standard = 1, Random = 34, Turn Zero = 35, ... */
    int64_t visibility; /* Failed = 0, Public = 1, Private = 2 */
    int64_t passcode; /* Success = passcode, Failed = -1, provide even when match is public */
};

struct C2SMatchCancel
{
    uint64_t length; /* = 9 */
    int64_t type; /* = 5 */
    int8_t unknown; /* = 0 */
};

struct S2CMatchCancelResult
{
    uint64_t length; /* = 16 */
    int64_t type; /* = 6 */
    int64_t result; /* Success = 1, Failed = 0 */
};

struct S2CMatchStart
{
    uint64_t length; /* = 48 */
    int64_t type; /* = 7 */
    int64_t clock; /* No Clock = 1, Short = 2, Medium = 3, Long = 4 */
    int64_t variant; /* Standard = 1, Random = 34, Turn Zero = 35, ... */
    uint64_t matchId; /* probably some auto increasing identifier of the match */
    int64_t color; /* yours, White = 0, Black = 1 */
    uint64_t messageId; /* probably some auto increasing identifier of the message */
};

/* type = 8 is never seen, why? */

struct S2COpponentLeft
{
    uint64_t length; /* = 9 */
    int64_t type; /* = 9 */
    int8_t unknown; /* = 0 */
};

struct C2SForfeit
{
    uint64_t length; /* = 9 */
    int64_t type; /* = 10 */
    int8_t unknown; /* = 0 */
};

/* C2S carries your action, S2C carries their action.
All judgments (capture, check, checkmate, clock, etc.) is performed locally.
Server will echo back with id added on every C2S action message.
A single header without action is considered an opponent timeout. */
struct C2SOrS2CAction
{
    uint64_t length; /* = 112 */
    int64_t type; /* = 11 */
    int64_t actionType; /* Move = 1, Undo Move = 2, Submit Moves = 3, Reset Puzzle (it does reset the whole game, be cautious!) = 4, Display Check Reason = 5, Header = 6 */
    int64_t color; /* White = 0, Black = 1 */
    uint64_t messageId; /* C2S = 0, S2C = probably some auto increasing identifier of the message */
    /* following ignored if actionType is not Move = 1 or Display Check Reason = 5 */
    int64_t srcL;
    int64_t srcT;
    int64_t srcBoardColor; /* White = 0, Black = 1 */
    /* following ignored if actionType is not Move = 1 */
    int64_t srcY; /* starts from 0 */
    int64_t srcX; /* starts from 0 */
    int64_t dstL;
    int64_t dstT;
    int64_t dstBoardColor; /* White = 0, Black = 1 */
    int64_t dstY; /* starts from 0 */
    int64_t dstX; /* starts from 0 */
};

struct C2SMatchListRequest
{
    uint64_t length; /* = 9 */
    int64_t type; /* = 12 */
    int8_t unknown; /* = 0 */
};

struct S2CMatchList
{
    uint64_t length; /* = 1008 */
    int64_t type; /* = 13 */
    int64_t unknown1; /* = 1 */
    int64_t color; /* Non-host = 0, Random = 1, White = 2, Black = 3 */
    int64_t clock; /* Non-host = 0, No Clock = 1, Short = 2, Medium = 3, Long = 4 */
    int64_t variant; /* Non-host = 0, Standard = 1, Random = 34, Turn Zero = 35, ... */
    int64_t passcode; /* Non-host = 0, Host = passcode */
    int64_t isHost; /* Non-host = 0, Host = 1 */

    struct PublicMatch
    {
        int64_t color; /* None = 0, Random = 1, White = 2, Black = 3 */
        int64_t clock; /* None = 0, No Clock = 1, Short = 2, Medium = 3, Long = 4 */
        int64_t variant; /* None = 0, Standard = 1, Random = 34, Turn Zero = 35, ... */
        int64_t passcode; /* None = 0, Some = passcode */
    } publicMatches[13];
    int64_t publicMatchesCount;

    struct ServerHistoryMatch
    {
        int64_t status; /* Completed = 0, In Progress = 1 */
        int64_t clock; /* No Clock = 1, Short = 2, Medium = 3, Long = 4 */
        int64_t variant; /* Standard = 1, Random = 34, Turn Zero = 35, ... */
        int64_t visibility; /* Public = 1, Private = 2 */
        int64_t secondsPassed;
    } serverHistoryMatches[13];
    int64_t serverHistoryMatchesCount;
};
