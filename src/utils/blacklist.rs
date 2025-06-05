use anyhow::{Error, Result, anyhow};
use crate::lookup::nojs;
use crate::format::get_country_format;

// Structure to store test case information
struct TestCase {
    phone: String,
    first_name: String,
    last_name: String,
}

// Get test case for a specific country code from format.json
fn get_test_case_for_country(country_code: &str) -> Result<TestCase, Error> {
    // Try to get format for this country
    let format = get_country_format(country_code)?;
    
    // Check if the country has blacklist information
    if let Some(blacklist) = format.blacklist {
        Ok(TestCase {
            phone: format!("{}{}", format.code, blacklist.phone),
            first_name: blacklist.first,
            last_name: blacklist.last,
        })
    } else {
        Err(anyhow!("No blacklist test data found for country code: {}. Blacklist verification is not possible.", country_code))
    }
}

pub async fn check_all_countries_blacklist(subnet: &str) -> Result<Vec<String>, Error> {
    // First load format data to get all countries
    let _ = crate::format::load_format_data()?;
    
    // Get all countries from format.json
    let all_countries = crate::format::get_all_countries()?;
    
    let mut blacklisted_countries = Vec::new();
    
    // Check each country
    for country_code in all_countries {
        // Verify this country has blacklist data before checking
        let has_blacklist = match crate::format::get_country_format(&country_code) {
            Ok(format) => format.blacklist.is_some(),
            Err(_) => false,
        };
        
        if has_blacklist {
            // Try to check if this subnet is blacklisted for this country
            match check_blacklist(subnet, &country_code).await {
                Ok(is_blacklisted) => {
                    if is_blacklisted {
                        blacklisted_countries.push(country_code.clone());
                        println!("❌ Subnet {} is blacklisted for country: {}", subnet, country_code);
                    } else {
                        println!("✅ Subnet {} is NOT blacklisted for country: {}", subnet, country_code);
                    }
                },
                Err(e) => {
                    eprintln!("Failed to check blacklist for country {}: {}", country_code, e);
                }
            }
            
            // Add a small delay between checks to avoid overwhelming the API
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
    
    Ok(blacklisted_countries)
}

// Check if the current subnet is blacklisted for a specific country code
pub async fn check_blacklist(subnet: &str, country_code: &str) -> Result<bool, Error> {    
    // Get test case for this specific country
    let test_case = match get_test_case_for_country(country_code) {
        Ok(tc) => tc,
        Err(e) => {
            // Just warning, not a fatal error
            eprintln!("WARNING: {}", e);
            // Return not blacklisted since we can't verify
            return Ok(false);
        }
    };
    
    // Create client with the provided subnet
    let client = crate::utils::create_client(Some(subnet), "");
    
    // Try the lookup with our known valid test phone number
    match nojs::lookup(&client, &test_case.phone, &test_case.first_name, &test_case.last_name).await {
        Ok(exists) => {
            if !exists {
                // If our known valid number returns false, the subnet is likely blacklisted
                return Ok(true); // is blacklisted
            } else {
                return Ok(false); // not blacklisted
            }
        },
        Err(e) => {
            // If we get an error other than rate limiting, consider it potentially blacklisted
            if e.to_string() == "ratelimited" {
                // Rate limiting doesn't necessarily mean blacklisted, try with a different IP
                return Err(anyhow!("Rate limited during blacklist check. Try again."));
            } else {
                return Err(anyhow!("Error during blacklist check: {}", e));
            }
        }
    }
}

// Verify subnet for a specific country
pub async fn verify_subnet_for_country(subnet: &str, country_code: &str, max_attempts: usize) -> Result<(), Error> {
    for attempt in 0..max_attempts {
        match check_blacklist(subnet, country_code).await {
            Ok(is_blacklisted) => {
                if is_blacklisted {
                    return Err(anyhow!("Subnet {} is blacklisted for country code {}. Please try a different subnet.", subnet, country_code));
                } else {
                    return Ok(());
                }
            },
            Err(e) => {
                if e.to_string().contains("Rate limited") && attempt < max_attempts - 1 {
                    // If rate limited and we have attempts left, wait and retry
                    println!("Rate limited during blacklist check. Retrying ({}/{})...", attempt + 1, max_attempts);
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    }
    
    Err(anyhow!("Failed to verify subnet after {} attempts", max_attempts))
}