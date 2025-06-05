use clap::{Parser, ValueEnum, command};

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    #[value(name = "quick")]
    Quick,   // Reads from file (formerly "QuickScan"/"File")
    #[value(name = "full")]
    Full,    // Generates based on country format (formerly "FullScan"/"OTF")
    #[value(name = "blacklist")]
    Blacklist,
    #[value(name = "email")]
    Email,   // Mode for email lookup
    #[value(name = "csv")]
    Csv,     // Process a CSV file with masked phone numbers
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum LookupType {
    #[value(name = "js")]
    Js,     // Use the JavaScript endpoint
    #[value(name = "nojs")]
    NoJS,    // Use the NoJS endpoint
}

#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
  ./gpb -m quick -c us -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -x \"80\" -w 1000
  ./gpb -m full -c sg -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -p \"910\" -w 200
  ./gpb -m full -c us -f \"John\" -l \"\" -s \"2605:dead:ffff::/48\" -p \"877\" -w 200
  ./gpb -m blacklist -c sg -s \"2605:dead:ffff::/48\"
  ./gpb -m blacklist -s \"2605:dead:ffff::/48\"
  ./gpb -m email -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -i \"emails.txt\" -w 200
  ./gpb -m full -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -M \"• (•••) •••-••-64\" -w 200
  ./gpb -m quick -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -M \"• (•••) •••-••-64\" -w 200
  ./gpb -m full -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -M \"+1••••••••46\" -w 200
  ./gpb -m full -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -M \"+14•••••3819\" -w 200
  ./gpb -m csv -s \"2605:dead:ffff::/48\" -i \"input.csv\" -w 200 -S
  ./gpb -m full -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -b \"YOUR_BOTGUARD_TOKEN_HERE\"
  ./gpb -m full -f \"John\" -l \"Smith\" -s \"2605:dead:ffff::/48\" -L njs

NOTES:
  - Valid hits are written to output.txt in the current directory
  - IPv6 subnet must support outbound connections
  - In blacklist mode, specify a country code to check that specific country, or omit to check all countries
  - In full mode, the tool automatically generates numbers based on the country format
  - In email mode, the tool reads emails from a file, normalizes Gmail addresses, and checks if they exist
  - The mask option (-M) can be used with any mode to identify country and suffix from a masked phone number
  - Two mask formats are supported:
    1. Google recovery hint format with dots for masked digits: \"• (•••) •••-••-64\"
    2. International format with + prefix: \"+1••••••••46\" (can auto-detect country code and extract prefix/suffix)
    
  For international format masks:
    - Country code will be automatically detected (e.g., +1 for US, +44 for UK)
    - Prefix can be extracted if visible (e.g., \"+14•••••3819\" has prefix \"4\")
    - Suffix will be extracted (e.g., \"+14•••••3819\" has suffix \"3819\")
    
  CSV mode:
    - Process a CSV file containing masked phone numbers to check
    - Format: maskednumber,firstname,lastname
    - Example: +972•••••••01,John,Smith
    - Results written to output.csv in the format: phone,firstname,lastname
    - Use -S flag to skip remaining matches after finding the first hit
    
  Botguard token (-b):
    - Manually specify a botguard token to use instead of automatic token refresh
    - If provided, the tool will not attempt to refresh the token
    
  Lookup type (-L):
    - Specify the lookup method: 'js' (JavaScript) or 'njs' (NoJS)
    - Default is 'js' which uses the JavaScript endpoint
    - Blacklist checks and verification always use NoJS regardless of this setting")]
pub struct Args {
    /// Operation mode: quick (read from file), full (generate based on country format), blacklist, email, or csv
    #[arg(value_enum, short = 'm', long, help_heading = "REQUIRED")]
    pub mode: Mode,

    /// Country code for input file or number generation (e.g., 'sg' for Singapore)
    /// Required when using quick mode, full mode, or blacklist mode.
    #[arg(short = 'c', long, required_if_eq_any([("mode", "Quick"), ("mode", "Full")]), help_heading = "REQUIRED")]
    pub country_code: Option<String>,
    
    /// Path to input file with phone numbers, emails, or CSV data (one per line)
    /// Required for quick, email, or csv mode
    #[arg(short = 'i', long, required_if_eq_any([("mode", "Email"), ("mode", "Csv")]), help_heading = "FILE INPUT")]
    pub input_file: Option<String>,
    
    /// First name for lookup (case sensitive)
    /// Used to verify the account holder's name.
    #[arg(short = 'f', long, default_value = "", help_heading = "LOOKUP INFO")]
    pub first_name: String,
    
    /// Last name for lookup (case sensitive, can be empty)
    /// Used to verify the account holder's name. Use empty string if no last name is needed.
    #[arg(short = 'l', long, default_value = "", help_heading = "LOOKUP INFO")]
    pub last_name: String,
    
    /// IPv6 subnet in CIDR notation (e.g., "2605:6400:5355::/48")
    /// Used for rotating IPs to avoid rate limiting.
    #[arg(short = 's', long, required = true, help_heading = "REQUIRED")]
    pub subnet: String,
    
    /// Phone number suffix to append (e.g., "00")
    /// Optional filter that appends digits to all numbers.
    #[arg(short = 'x', long, help_heading = "OPTIONAL")]
    pub suffix: Option<String>,
    
    /// Phone number prefix that includes the area code (e.g., "910" for Singapore mobile)
    /// When used with -c sg, -p 910 would generate numbers like +65910xxxxx
    #[arg(short = 'p', long, help_heading = "FULL MODE")]
    pub prefix: Option<String>,
    
    /// Number of digits for generated numbers (overrides country format)
    /// Required when country format is not available.
    #[arg(short = 'd', long, help_heading = "FULL MODE")]
    pub digits: Option<usize>,
    
    /// Masked phone number pattern (e.g., "• (•••) •••-••-64" or "+1••••••••46")
    /// Used to identify country and extract suffix from masked numbers.
    /// Can be used with any mode to automatically determine country code and suffix.
    /// Supports two formats:
    /// 1. Standard format with dots for masked digits
    /// 2. International format with + prefix (can auto-detect country and extract prefix/suffix)
    #[arg(short = 'M', long = "mask", help_heading = "OPTIONAL")]
    pub masked_phone: Option<String>,
    
    /// Number of worker threads to use
    /// More threads can improve performance but may increase rate limiting.
    #[arg(short = 'w', long, default_value_t = 100, help_heading = "OPTIONAL")]
    pub workers: usize,
    
    /// Skip remaining potential matches after finding the first hit (CSV mode only)
    /// By default, all potential matches are found and joined with : in the output
    #[arg(short = 'S', long = "skip", default_value_t = false, help_heading = "CSV MODE")]
    pub skip_after_hit: bool,

    /// Manually specify a botguard token instead of using automatic token refresh
    /// If provided, the automatic token refresh mechanism will be disabled
    #[arg(short = 'b', long, help_heading = "OPTIONAL")]
    pub botguard_token: Option<String>,
    
    /// Lookup type: js (JavaScript) or nojs (NoJS)
    /// Specifies which endpoint to use for lookups
    /// Blacklist checks and verification always use NoJS regardless of this setting
    #[arg(value_enum, short = 'L', long, default_value = "nojs", help_heading = "OPTIONAL")]
    pub lookup_type: LookupType,
}