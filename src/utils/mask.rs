use std::collections::HashMap;
use std::fs;
use anyhow::{Result, Error, anyhow};
use serde::{Deserialize, Serialize};
use lazy_static::lazy_static;
use std::sync::RwLock;
use crate::format::{get_country_format, get_all_countries};

#[derive(Debug, Deserialize, Serialize)]
pub struct MaskData {
    #[serde(flatten)]
    pub masks: HashMap<String, Vec<String>>,
}

// Structure to store all extracted information from a masked phone number
#[derive(Debug, Clone)]
pub struct MaskedPhoneInfo {
    pub country_code: String,   // Country code (e.g., "us", "sg")
    pub suffix: String,         // Last few visible digits
    pub prefix: Option<String>, // Visible digits after country code
    pub infix: Option<String>,  // 2 digits at specific position
}

lazy_static! {
    static ref MASK_DATA: RwLock<Option<MaskData>> = RwLock::new(None);
}

/// Load and parse the mask.json file
pub fn load_mask_data() -> Result<(), Error> {
    let mask_path = "data/mask.json";
    
    // Check if file exists
    if !std::path::Path::new(mask_path).exists() {
        return Err(anyhow!("Mask data file not found: {}", mask_path));
    }
    
    // Read and parse the JSON
    let mask_json = fs::read_to_string(mask_path)?;
    let mask_data: MaskData = serde_json::from_str(&mask_json)?;
    
    // Store in the global variable
    let mut data = MASK_DATA.write().unwrap();
    *data = Some(mask_data);
    
    Ok(())
}

/// Get countries that match a given mask pattern
pub fn get_countries_for_mask(mask_pattern: &str) -> Result<Vec<String>, Error> {
    // Try to load mask data if not already loaded
    if MASK_DATA.read().unwrap().is_none() {
        load_mask_data()?;
    }
    
    // Get the data
    let data = MASK_DATA.read().unwrap();
    
    if let Some(data) = &*data {
        // Look for matching mask pattern
        if let Some(countries) = data.masks.get(mask_pattern) {
            return Ok(countries.clone());
        }
    }
    
    Err(anyhow!("No matching mask pattern found: {}. Make sure mask.json contains this pattern.", mask_pattern))
}

/// Extract the suffix from a masked phone number
/// Returns (suffix, suffix_length)
pub fn extract_suffix_from_mask(masked_phone: &str) -> Result<(String, usize), Error> {
    // Extract non-masked (non-•) digits from the end of the string
    let mut suffix = String::new();
    let mut count = 0;
    
    // Process the masked phone from the end
    for c in masked_phone.chars().rev() {
        if c.is_digit(10) {
            suffix.insert(0, c);
            count += 1;
        } else if c == '•' {
            // Stop when we hit a mask character
            break;
        }
        // Ignore other characters like spaces, dashes, etc.
    }
    
    if suffix.is_empty() {
        return Err(anyhow!("No suffix digits found in the masked phone number"));
    }
    
    Ok((suffix, count))
}

/// Extract infix from a masked phone number in international format
/// Returns (infix, infix_length) or None if no infix is found
pub fn extract_infix_from_mask(masked_phone: &str) -> Option<(String, usize)> {
    // The infix is 2 digits that are 6 and 5 characters from the end
    // Check if the length is sufficient
    if masked_phone.len() < 6 {
        return None;
    }
    
    // Extract the potential infix (6th and 5th characters from the end)
    let chars: Vec<char> = masked_phone.chars().collect();
    let potential_infix = chars[chars.len().saturating_sub(6)..chars.len().saturating_sub(4)].iter().collect::<String>();
    
    // Check if both characters in the potential infix are digits
    if potential_infix.chars().all(|c| c.is_digit(10)) && potential_infix.len() == 2 {
        return Some((potential_infix, 2));
    }
    
    None
}

/// Extract prefix from a masked phone number when the country code is known
/// Returns (prefix, prefix_length)
pub fn extract_prefix_from_mask(masked_phone: &str, country_code: &str) -> Result<(String, usize), Error> {
    // Strip any non-digit characters from country code for comparison
    let country_code = country_code.chars().filter(|c| c.is_digit(10)).collect::<String>();
    
    // Find where the country code ends in the masked phone
    let mut prefix = String::new();
    let mut prefix_started = false;
    let mut code_chars_matched = 0;
    let mut count = 0;
    
    // Skip any + sign at the beginning
    let masked_chars: Vec<char> = masked_phone.chars().filter(|c| *c != '+').collect();

    // First match the country code
    for c in masked_chars.iter() {
        if code_chars_matched < country_code.len() {
            // Still matching country code
            if c.is_digit(10) && country_code.chars().nth(code_chars_matched) == Some(*c) {
                code_chars_matched += 1;
            }
            continue; // Skip to next character
        } else if !prefix_started {
            // Country code matched, start collecting prefix digits
            prefix_started = true;
        }
        
        // Now collect prefix digits until we hit a mask character
        if *c == '•' {
            break; // End of prefix
        } else if c.is_digit(10) {
            prefix.push(*c);
            count += 1;
        }
    }
    
    Ok((prefix, count))
}

/// Create a fully masked pattern by replacing all digits with dots
fn create_fully_masked_pattern(pattern: &str) -> String {
    pattern.chars()
        .map(|c| if c.is_digit(10) { '•' } else { c })
        .collect()
}

// Extract visible country code from an international format masked phone number
fn extract_visible_country_code(masked_phone: &str) -> String {
    let mut visible_country_code = String::new();
    let mut i = 1; // Start after the plus sign
    
    while i < masked_phone.len() {
        let c = masked_phone.chars().nth(i).unwrap();
        if c.is_digit(10) {
            visible_country_code.push(c);
        } else if c == '•' {
            // We've reached the masked part
            break;
        }
        i += 1;
    }
    
    visible_country_code
}

/// Process a masked phone number to extract all information (country, suffix, prefix, infix)
/// This consolidated function replaces the separate extractions
pub fn extract_info_from_masked_phone(masked_phone: &str, explicit_country_code: Option<&str>) -> Result<MaskedPhoneInfo, Error> {
    // Make sure format data is loaded
    crate::format::load_format_data()?;
    
    // Detect if this is the international format with + sign
    let is_international = masked_phone.starts_with("+");
    
    // First, determine the country code
    let country_code = if let Some(cc) = explicit_country_code {
        // If explicitly provided, use that
        cc.to_string()
    } else if is_international {
        // For international format, try to determine from the visible digits
        process_international_country_code(masked_phone)?
    } else {
        // For standard format, use mask pattern matching
        let fully_masked = create_fully_masked_pattern(masked_phone);
        let countries = get_countries_for_mask(&fully_masked)?;
        
        // If multiple countries match, return an error with the list
        if countries.len() > 1 {
            let countries_list = countries.join(", ");
            return Err(anyhow!("Multiple countries match this mask pattern: {}", countries_list));
        }
        
        // If exactly one country matches, use it
        countries.first()
            .ok_or_else(|| anyhow!("No countries found for this mask pattern"))?
            .clone()
    };
    
    // Extract the suffix (common for all formats)
    let (suffix, _) = extract_suffix_from_mask(masked_phone)?;
    
    // Extract prefix and infix if this is an international format
    let (prefix, infix) = if is_international {
        // Get the numeric country code from the country code string
        let format = get_country_format(&country_code)?;
        
        // Extract prefix
        let prefix = match extract_prefix_from_mask(masked_phone, &format.code) {
            Ok((p, _)) if !p.is_empty() => Some(p),
            _ => None
        };
        
        // Extract infix
        let infix = extract_infix_from_mask(masked_phone)
            .map(|(i, _)| i);
        
        (prefix, infix)
    } else {
        // For standard format, don't attempt to extract prefix/infix
        (None, None)
    };
    
    Ok(MaskedPhoneInfo {
        country_code,
        suffix,
        prefix,
        infix,
    })
}

// Helper function to determine country code from international format
fn process_international_country_code(masked_phone: &str) -> Result<String, Error> {
    // Extract digits from the beginning of the masked phone after the plus sign
    let visible_digits = extract_visible_country_code(masked_phone);
    
    // If no visible digits, we can't determine country code
    if visible_digits.is_empty() {
        return Err(anyhow!("Cannot determine country code from the masked phone number. No visible digits after the plus sign."));
    }
    
    // Get all possible countries from format.json
    let all_countries = get_all_countries()?;
    let mut matching_countries = Vec::new();
    
    // First try: look for direct country code matches
    for country in &all_countries {
        if let Ok(format) = get_country_format(&country) {
            if visible_digits.starts_with(&format.code) {
                matching_countries.push((country.to_string(), format.code.clone()));
            }
        }
    }
    
    // If no matches yet, try more complex matching by checking various prefixes
    if matching_countries.is_empty() {
        for prefix_len in 1..visible_digits.len() {
            let potential_country_code = &visible_digits[0..prefix_len];
            
            for country in &all_countries {
                if let Ok(format) = get_country_format(&country) {
                    if format.code == potential_country_code {
                        // Found a country code match, now check if remaining digits might be part of area code
                        let potential_area_code = &visible_digits[prefix_len..];
                        
                        // Check if this area code exists for this country, or if area codes aren't specified
                        if format.area_codes.is_empty() || 
                           format.area_codes.iter().any(|ac| ac.starts_with(potential_area_code)) {
                            matching_countries.push((country.to_string(), format.code.clone()));
                        }
                    }
                }
            }
            
            // If we found matches, break
            if !matching_countries.is_empty() {
                break;
            }
        }
    }
    
    // Process results
    if matching_countries.is_empty() {
        return Err(anyhow!("No country found with code matching +{}. Please check the masked phone number format.", visible_digits));
    } else if matching_countries.len() > 1 {
        // Multiple matches - list them for the user
        let countries_list = matching_countries.iter()
            .map(|(country, code)| format!("{} (+{})", country, code))
            .collect::<Vec<String>>()
            .join(", ");
            
        return Err(anyhow!("Multiple countries match this code: {}. Please specify a country code with -c.", countries_list));
    }
    
    // We have a unique match
    let (country, _) = &matching_countries[0];
    Ok(country.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_info_from_masked_phone() {
        let _ = crate::format::load_format_data().unwrap();
        let _ = load_mask_data().unwrap();
        
        // Test cases for different masked phone formats
        let test_cases = [
            // International format with visible country code, prefix, infix, suffix
            ("+4478••02••17", None, "gb", Some("78"), Some("02"), "17"),
            // International format with just country code and suffix
            ("+1••••••••46", None, "us", None, None, "46"),
            // Traditional format
            ("• (•••) •••-••-64", None, "ru", None, None, "64"),
            // Multiple country possibility with explicit override
            ("•••• ••••20", Some("jp"), "jp", None, None, "20"),
        ];
        
        for (input, explicit_cc, expected_cc, expected_prefix, expected_infix, expected_suffix) in test_cases {
            match extract_info_from_masked_phone(input, explicit_cc) {
                Ok(info) => {
                    assert_eq!(info.country_code.to_lowercase(), expected_cc, "Wrong country for {}", input);
                    assert_eq!(info.suffix, expected_suffix, "Wrong suffix for {}", input);
                    
                    if let Some(expected) = expected_prefix {
                        assert_eq!(info.prefix.as_deref(), Some(expected), "Wrong prefix for {}", input);
                    } else {
                        assert!(info.prefix.is_none(), "Should have no prefix for {}", input);
                    }
                    
                    if let Some(expected) = expected_infix {
                        assert_eq!(info.infix.as_deref(), Some(expected), "Wrong infix for {}", input);
                    } else {
                        assert!(info.infix.is_none(), "Should have no infix for {}", input);
                    }
                    
                    println!("Successfully extracted info from '{}'", input);
                },
                Err(e) => {
                    if explicit_cc.is_none() && input == "•••• ••••20" {
                        // This should fail without explicit country code
                        assert!(e.to_string().contains("Multiple countries"), 
                               "Should error for ambiguous pattern");
                    } else {
                        panic!("Failed to extract info from {}: {}", input, e);
                    }
                }
            }
        }
    }
    
    #[test]
    fn test_load_mask_data() {
        let result = load_mask_data();
        assert!(result.is_ok(), "Failed to load mask data");
    }
    
    #[test]
    fn test_extract_infix_from_mask() {
        // Test cases where infix should be detected
        let test_cases_with_infix = [
            ("+141••02••00", "02"),
            ("+1••••10••23", "10"),
            ("+33••••64••12", "64")
        ];
        
        for (input, expected_infix) in test_cases_with_infix {
            match extract_infix_from_mask(input) {
                Some((infix, len)) => {
                    assert_eq!(infix, expected_infix, "Wrong infix extracted from {}", input);
                    assert_eq!(len, 2, "Infix length should be 2 for {}", input);
                    println!("Successfully extracted infix '{}' from '{}'", infix, input);
                },
                None => panic!("Failed to extract infix from {}", input)
            }
        }
        
        // Test cases where infix should not be detected
        let test_cases_without_infix = [
            "+44••••••54••",
            "+91•••••78••",
            "+141••0•••00",  // Contains dot in infix position
            "+44•••••3••00", // Contains dot in infix position
            "+44••••••••53", // No digits in infix position
            "+4••••••00",    // Too short
            "+49•••••",      // Too short
            "+1••••"         // Too short
        ];
        
        for input in test_cases_without_infix {
            match extract_infix_from_mask(input) {
                Some((infix, _)) => {
                    panic!("Should not have extracted infix '{}' from '{}'", infix, input);
                },
                None => {
                    println!("Correctly detected no infix in '{}'", input);
                }
            }
        }
    }
    
    #[test]
    fn test_get_countries_for_mask() {
        let _ = load_mask_data();
        
        // Test with a known mask pattern
        let countries = get_countries_for_mask("•••-••••-••••").unwrap();
        assert!(countries.contains(&"jp".to_string()) || countries.contains(&"kr".to_string()),
                "Expected JP or KR for mask pattern •••-••••-••••");
        
        // Test with a multi-country mask
        let countries = get_countries_for_mask("•••• ••••••").unwrap();
        assert!(countries.len() > 1, "Expected multiple countries for mask pattern •••• ••••••");
    }
    
    #[test]
    fn test_extract_suffix() {
        let test_cases = [
            ("• (•••) •••-••-64", "64", 2),
            ("•• ••• ••••-789", "789", 3),
            ("••••-•••-123", "123", 3),
            ("• •• ••• •• ••", "", 0),
            ("+1•••••••46", "46", 2)
        ];
        
        for (input, expected_suffix, expected_len) in test_cases {
            let result = extract_suffix_from_mask(input);
            if expected_suffix.is_empty() {
                assert!(result.is_err(), "Expected error for {}", input);
            } else {
                let (suffix, len) = result.unwrap();
                assert_eq!(suffix, expected_suffix, "Wrong suffix for {}", input);
                assert_eq!(len, expected_len, "Wrong suffix length for {}", input);
            }
        }
    }
    
    #[test]
    fn test_create_fully_masked_pattern() {
        let test_cases = [
            ("• (•••) •••-••-64", "• (•••) •••-••-••"),
            ("••• ••••-789", "••• ••••-•••"),
            ("1234", "••••"),
            ("+1••••••••46", "+•••••••••••")
        ];
        
        for (input, expected) in test_cases {
            let result = create_fully_masked_pattern(input);
            assert_eq!(result, expected, "Wrong fully masked pattern for {}", input);
        }
    }
    
    #[test]
    fn test_extract_prefix() {
        let _ = crate::format::load_format_data().unwrap();
        
        let test_cases = [
            ("+14•••••3819", "1", "4", 1),
            ("+1••••••••46", "1", "", 0),
            ("+6591•••••••", "65", "91", 2)
        ];
        
        for (input, country_code, expected_prefix, expected_len) in test_cases {
            let result = extract_prefix_from_mask(input, country_code);
            assert!(result.is_ok(), "Failed to extract prefix for {}", input);
            let (prefix, len) = result.unwrap();
            assert_eq!(prefix, expected_prefix, "Wrong prefix for {}", input);
            assert_eq!(len, expected_len, "Wrong prefix length for {}", input);
        }
    }
    
    #[test]
    fn test_extract_visible_country_code() {
        let test_cases = [
            ("+1••••••••46", "1"),
            ("+44•••••••12", "44"),
            ("+447•••••221", "447"),
            ("+••••••••••", ""),
        ];
        
        for (input, expected) in test_cases {
            let result = extract_visible_country_code(input);
            assert_eq!(result, expected, "Wrong visible country code for {}", input);
        }
    }

    #[test]
    fn test_extract_visible_country_code_with_area_code() {
        let test_cases = [
            ("+4478••••••17", "4478"),
            ("+1212•••••••", "1212"),
            ("+61412•••••", "61412"),
            ("+••••••••", ""),
        ];
        
        for (input, expected) in test_cases {
            let result = extract_visible_country_code(input);
            assert_eq!(result, expected, "Wrong visible digits for {}", input);
        }
    }
    
    // Test cases for masked phone numbers with infix
    #[test]
    fn test_international_format_with_infix() {
        let _ = crate::format::load_format_data().unwrap();
        
        let result = extract_infix_from_mask("+141••02••00");
        assert!(result.is_some());
        let (infix, len) = result.unwrap();
        assert_eq!(infix, "02");
        assert_eq!(len, 2);
        
        // Test with a different index position
        let result = extract_infix_from_mask("+44•••••89•1");
        assert!(result.is_none());
        
        // Test with no infix (dots in the infix position)
        let result = extract_infix_from_mask("+44•••••••53");
        assert!(result.is_none());
        
        // Test with partial infix (only one digit in infix position)
        let result = extract_infix_from_mask("+44••••5••53");
        assert!(result.is_none());
    }
}