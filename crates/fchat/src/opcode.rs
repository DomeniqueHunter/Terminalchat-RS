//! Three letter operation codes of the F-Chat websocket protocol.

// Global moderation
pub const ACCOUNT_BAN: &str = "ACB";
pub const ACCOUNT_UNBAN: &str = "UBN";
pub const ACCOUNT_TIMEOUT: &str = "TMO";
pub const SERVER_KICK: &str = "KIK";
pub const PROMOTE_GLOBAL_OP: &str = "AOP";
pub const DEMOTE_GLOBAL_OP: &str = "DOP";
pub const LIST_ALTS: &str = "AWC";
pub const BROADCAST: &str = "BRO";
pub const REWARD: &str = "RWD";
pub const RELOAD_SERVER_CONFIG: &str = "RLD";

// Channels
pub const LIST_OFFICIAL_CHANNELS: &str = "CHA";
pub const LIST_PRIVATE_CHANNELS: &str = "ORS";
pub const INITIAL_CHANNEL_DATA: &str = "ICH";
pub const TIMEOUT: &str = "CTU";
pub const KICK: &str = "CKU";
pub const BAN: &str = "CBU";
pub const UNBAN: &str = "CUB";
pub const BANLIST: &str = "CBL";
pub const CREATE_PRIVATE_CHANNEL: &str = "CCR";
pub const CREATE_OFFICIAL_CHANNEL: &str = "CRC";
pub const CHANNEL_DESCRIPTION: &str = "CDS";
pub const SET_CHANNEL_OWNER: &str = "CSO";
pub const PROMOTE_OP: &str = "COA";
pub const DEMOTE_OP: &str = "COR";
pub const INVITE: &str = "CIU";
pub const LIST_OPS: &str = "COL";
pub const SET_CHANNEL_MODE: &str = "RMO";
pub const SET_PRIVATE_CHANNEL_STATE: &str = "RST";
pub const ROLEPLAY_AD: &str = "LRP";
pub const CHANNEL_MESSAGE: &str = "MSG";

// Characters
pub const JOIN_CHANNEL: &str = "JCH";
pub const GONE_OFFLINE: &str = "FLN";
pub const PROFILE_DATA: &str = "PRD";
pub const PROFILE: &str = "PRO";
pub const KINKS_DATA: &str = "KID";
pub const KINKS: &str = "KIN";
pub const LEAVE_CHANNEL: &str = "LCH";
pub const LIST_CHARACTERS: &str = "LIS";
pub const USER_CONNECTED: &str = "NLN";
pub const FRIENDS_LIST: &str = "FRL";
pub const PRIVATE_MESSAGE: &str = "PRI";
pub const IGNORE: &str = "IGN";
pub const TYPING: &str = "TPN";
pub const STATUS: &str = "STA";

// General
pub const LIST_GLOBAL_OPS: &str = "ADL";
pub const NUMBER_OF_CONNECTED_USERS: &str = "CON";
pub const ERROR: &str = "ERR";
pub const SEARCH: &str = "FKS";
pub const SERVER_HELLO: &str = "HLO";
pub const IDENTIFY: &str = "IDN";
pub const PING: &str = "PIN";
pub const ROLL: &str = "RLL";
pub const REPORT: &str = "SFC";
pub const SITE_EVENT: &str = "RTB";
pub const SYSTEM_MESSAGE: &str = "SYS";
pub const UPTIME: &str = "UPT";
pub const VARIABLES: &str = "VAR";
