use std::collections::HashMap;
use std::fs;
use anyhow::{Result, Error, anyhow};
use serde::{Deserialize, Serialize};
use lazy_static::lazy_static;
use std::sync::RwLock;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BlacklistInfo {
    pub first: String,
    pub last: String,
    pub phone: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum Digits {
    Single(usize),
    Multiple(Vec<usize>),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CountryFormat {
    pub code: String,
    #[serde(default)]
    pub area_codes: Vec<String>,
    pub digits: Option<Digits>,
    pub blacklist: Option<BlacklistInfo>,
}

pub type FormatData = HashMap<String, CountryFormat>;

lazy_static! {
    static ref FORMAT_DATA: RwLock<Option<FormatData>> = RwLock::new(None);
}

/// Load and parse the format.json file
pub fn load_format_data() -> Result<(), Error> {
    let format_path = "data/format.json";
    
    // Check if file exists
    if !std::path::Path::new(format_path).exists() {
        return Err(anyhow!("Format data file not found: {}", format_path));
    }
    
    // Read and parse the JSON
    let format_json = fs::read_to_string(format_path)?;
    let format_data: FormatData = serde_json::from_str(&format_json)?;
    
    // Store in the global variable
    let mut data = FORMAT_DATA.write().unwrap();
    *data = Some(format_data);
    
    Ok(())
}

/// Get country format information
pub fn get_country_format(country_code: &str) -> Result<CountryFormat, Error> {
    // Convert to lowercase and remove any potential spaces
    let country_code = country_code.trim().to_lowercase();
    
    // Try to load format data if not already loaded
    if FORMAT_DATA.read().unwrap().is_none() {
        load_format_data()?;
    }
    
    // Get the data
    let data = FORMAT_DATA.read().unwrap();
    
    if let Some(data) = &*data {
        // Try direct lookup by country code
        if let Some(format) = data.get(&country_code) {
            return Ok(format.clone());
        }
        
        // Try lookup by calling code
        for (_, format) in data.iter() {
            if format.code == country_code {
                return Ok(format.clone());
            }
        }
    }
    
    Err(anyhow!("No format data found for country code: {}. Make sure format.json contains this country.", country_code))
}

/// Get the digits for a country format
pub fn get_digits_for_country(format: &CountryFormat) -> Result<Vec<usize>, Error> {
    if let Some(digits) = &format.digits {
        match digits {
            Digits::Single(d) => Ok(vec![*d]),
            Digits::Multiple(v) => Ok(v.clone()),
        }
    } else {
        Err(anyhow!("No digit information found for country code: {}", format.code))
    }
}

/// Get a list of all countries in the format data
pub fn get_all_countries() -> Result<Vec<String>, Error> {
    // Try to load format data if not already loaded
    if FORMAT_DATA.read().unwrap().is_none() {
        load_format_data()?;
    }
    
    // Get the data
    let data = FORMAT_DATA.read().unwrap();
    
    if let Some(data) = &*data {
        // Extract all country codes
        let countries: Vec<String> = data.keys().cloned().collect();
        return Ok(countries);
    }
    
    Err(anyhow!("Format data not available"))
}


// First, let's modify the PhoneNumberGenerator struct to add the infix field
pub struct PhoneNumberGenerator {
    country_code: String,                    // Country calling code (e.g., "1" for US, "65" for SG)
    selected_area_codes: Vec<String>,        // Selected area codes based on prefix filter
    digits_per_number: usize,                // Number of digits to generate after country+area code
    prefix: Option<String>,                  // User-specified prefix (may override or extend area code)
    suffix_filter: Option<String>,           // Numbers must end with this suffix
    infix_filter: Option<String>,            // Numbers must have this infix at specific position from the end
    current_area_code_idx: usize,            // Current area code index
    current_number: u64,                     // Current number in sequence
    max_numbers_per_segment: u64,            // Maximum numbers per segment
    has_more: bool,                          // Whether more numbers can be generated
    remaining_area_code_parts: Vec<String>,  // Remaining parts of area codes after partial prefix match
}

// Now modify the new() method to accept an infix parameter
impl PhoneNumberGenerator {
    pub fn new(
        country_format: &CountryFormat,
        prefix: Option<String>,
        suffix_filter: Option<String>,
        infix_filter: Option<String>,
        digit_override: Option<usize>,
    ) -> Result<Self, Error> {
        let country_code = country_format.code.clone();
        
        // Get the format-defined digits (how many digits in a complete number for this country)
        let format_digits = if let Some(d) = digit_override {
            d
        } else {
            match get_digits_for_country(country_format) {
                Ok(digit_lengths) => {
                    if digit_lengths.is_empty() {
                        return Err(anyhow!("No digit length specified for country code: {}. Check format.json", country_code));
                    }
                    // Use the first/minimum digit length
                    *digit_lengths.iter().min().unwrap()
                },
                Err(e) => return Err(e)
            }
        };
        
        // Calculate the standard area code length for this country
        let standard_area_code_len = if !country_format.area_codes.is_empty() {
            // Most countries have consistent area code lengths, so we can use the first one
            // as a reference
            country_format.area_codes[0].len()
        } else {
            0 // No area codes
        };
        
        // Filter available area codes and calculate remaining parts based on user-provided prefix
        let (selected_area_codes, remaining_area_code_parts, digits_to_generate) = 
            if let Some(p) = &prefix {
                // Using a user-defined prefix:
                // 1. If prefix is shorter than or equal to expected area code length, 
                //    use it to filter area codes
                // 2. If prefix is longer than area codes, split it to extract area code 
                //    and starting digits
                
                // Check if any area codes are defined for this country
                if country_format.area_codes.is_empty() {
                    // No area codes specified, use empty string as the only "area code"
                    // and generate numbers with the full prefix
                    (
                        vec!["".to_string()],
                        vec!["".to_string()], 
                        format_digits.saturating_sub(p.len())
                    )
                } else {
                    if p.len() <= standard_area_code_len {
                        // Prefix is shorter than or equal to typical area code length
                        // Filter area codes that start with this prefix
                        let mut matching_codes = Vec::new();
                        let mut remaining_parts = Vec::new();
                        
                        for ac in &country_format.area_codes {
                            if ac.starts_with(p) {
                                matching_codes.push(ac.clone());
                                // Store the remaining part of the area code after the prefix
                                remaining_parts.push(ac[p.len()..].to_string());
                            }
                        }
                        
                        if matching_codes.is_empty() {
                            return Err(anyhow!("No matching area codes found for prefix '{}'", p));
                        }
                        
                        (matching_codes, remaining_parts, format_digits)
                    } else {
                        // Prefix is longer than area code, need to extract area code and remaining digits
                        // Extract the first N characters as the area code part
                        let area_code_part = &p[0..std::cmp::min(p.len(), standard_area_code_len)];
                        
                        // Filter area codes matching the extracted part
                        let mut matching_codes = Vec::new();
                        let mut remaining_parts = Vec::new();
                        
                        for ac in &country_format.area_codes {
                            if ac.starts_with(area_code_part) {
                                matching_codes.push(ac.clone());
                                // For longer prefixes, there's no remaining part (it's already in the prefix)
                                remaining_parts.push("".to_string());
                            }
                        }
                        
                        if matching_codes.is_empty() {
                            return Err(anyhow!("No matching area codes found for prefix '{}'", area_code_part));
                        }
                        
                        // Important: We need to calculate digits correctly here based on:
                        // 1. The total format digits
                        // 2. How many digits we're already specifying in the prefix beyond the area code
                        let extra_prefix_digits = p.len().saturating_sub(standard_area_code_len);
                        
                        // Calculate remaining digits to generate, compensating for the extra prefix digits
                        let remaining_digits = format_digits.saturating_sub(extra_prefix_digits);
                        
                        (
                            matching_codes,
                            remaining_parts,
                            remaining_digits
                        )
                    }
                }
            } else {
                // No user prefix - use all available area codes
                if country_format.area_codes.is_empty() {
                    return Err(anyhow!("No area codes specified for country code: {}. Check format.json", country_code));
                }
                
                // No remaining parts when using full area codes
                let empty_parts = vec!["".to_string(); country_format.area_codes.len()];
                
                (country_format.area_codes.clone(), empty_parts, format_digits)
            };
        
        // Calculate max numbers per segment based on digits to generate
        let max_numbers = 10_u64.pow(digits_to_generate as u32);
        
        // Apply suffix and infix filter adjustments if needed
        let effective_max = if let Some(suffix) = &suffix_filter {
            // If we have a suffix filter, we need to adjust our generation approach
            // Only about 1 in 10^suffix.len() numbers will end with the suffix
            // So we'll pre-calculate the matching numbers
            if suffix.len() > digits_to_generate {
                return Err(anyhow!("Suffix '{}' is longer than available digits to generate ({})", 
                                  suffix, digits_to_generate));
            }
            
            // For suffix filtering, we'll generate only the prefix part
            // and append the suffix
            max_numbers / 10_u64.pow(suffix.len() as u32)
        } else {
            max_numbers
        };
        
        Ok(Self {
            country_code,
            selected_area_codes,
            remaining_area_code_parts,
            digits_per_number: digits_to_generate,
            prefix,
            suffix_filter,
            infix_filter,
            current_area_code_idx: 0,
            current_number: 0,
            max_numbers_per_segment: effective_max,
            has_more: true,
        })
    }
    
    /// Get the next phone number in the sequence
    pub fn next(&mut self) -> Option<String> {
        if !self.has_more {
            return None;
        }
        
        // Keep generating numbers until we find one that matches our infix filter (if any)
        loop {
            // Use current area code and number
            let area_code = &self.selected_area_codes[self.current_area_code_idx];
            let remaining_area_code_part = &self.remaining_area_code_parts[self.current_area_code_idx];
            
            // Calculate the current number
            let formatted_number = if let Some(suffix) = &self.suffix_filter {
                // If we have a suffix, we'll generate the prefix part
                // and append the suffix
                let suffix_len = suffix.len();
                let prefix_len = self.digits_per_number - suffix_len;
                
                // Format the prefix part with proper padding
                if prefix_len > 0 {
                    format!("{}{:0width$}{}", remaining_area_code_part, self.current_number, suffix, width = prefix_len)
                } else {
                    // If we don't need to generate any additional digits, just use the remaining area code part and suffix
                    format!("{}{}", remaining_area_code_part, suffix)
                }
            } else {
                // No suffix - format the full number with proper padding
                format!("{}{:0width$}", remaining_area_code_part, self.current_number, width = self.digits_per_number)
            };
            
            // Increment for next call
            self.current_number += 1;
            if self.current_number >= self.max_numbers_per_segment {
                self.current_number = 0;
                self.current_area_code_idx += 1;
                
                if self.current_area_code_idx >= self.selected_area_codes.len() {
                    self.has_more = false;
                    // If we've run out of numbers and haven't found a match yet, return None
                    if self.infix_filter.is_some() {
                        return None;
                    }
                }
            }
            
            // Build the full phone number
            let mut phone = format!("{}", self.country_code);
            
            if let Some(p) = &self.prefix {
                // When prefix is provided, use it directly
                phone.push_str(p);
            } else {
                // Otherwise use the area code
                phone.push_str(area_code);
            }
            
            // Append the formatted number
            phone.push_str(&formatted_number);
            
            // Check if the number matches the infix filter
            if let Some(infix) = &self.infix_filter {
                // Check if the phone is long enough
                if phone.len() >= 6 {
                    // Extract the infix part (6th and 5th characters from the end)
                    let potential_infix = &phone[phone.len() - 6..phone.len() - 4];
                    
                    // If it matches our infix filter, return this number
                    if potential_infix == infix {
                        return Some(phone);
                    }
                    
                    // If this number doesn't match the infix, try the next one
                    continue;
                } else {
                    // Phone number is too short for infix filtering, skip it
                    continue;
                }
            }
            
            // If no infix filtering or we found a match, return the number
            return Some(phone);
        }
    }
    
    /// Estimate total count of numbers that will be generated
    pub fn estimate_total(&self) -> u64 {
        let area_code_count = self.selected_area_codes.len() as u64;
        let numbers_per_area_code = self.max_numbers_per_segment;
        
        // Calculate the total based on area codes and numbers per area code
        let total = area_code_count * numbers_per_area_code;
        
        // If we have an infix filter, adjust the estimate
        // For an infix of length 2, approximately 1 in 100 numbers will match
        if let Some(infix) = &self.infix_filter {
            // Each digit in the infix reduces the probability by a factor of 10
            let infix_factor = 10_u64.pow(infix.len() as u32);
            
            // Return the adjusted total, ensuring we don't return zero
            std::cmp::max(1, total / infix_factor)
        } else {
            total
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_format_loading() {
        let result = load_format_data();
        assert!(result.is_ok(), "Failed to load format data");
        
        let sg_format = get_country_format("sg");
        assert!(sg_format.is_ok(), "Failed to get SG format");
        
        let sg = sg_format.unwrap();
        assert_eq!(sg.code, "65");
        
        let area_codes = sg.area_codes;
        assert!(area_codes.contains(&"8".to_string()), "SG should have area code 8");
        assert!(area_codes.contains(&"9".to_string()), "SG should have area code 9");
    }

    #[test]
    fn test_specific_number_generation_us() {
        // Test case 1: Ensure US number 16477014383 is generated with prefix 647 and suffix 383
        let _ = load_format_data();
        let us_format = get_country_format("us").unwrap();
        
        // Create generator for US with prefix and suffix
        let mut gen = PhoneNumberGenerator::new(
            &us_format,
            Some("646".to_string()),  // Prefix
            Some("583".to_string()),  // Suffix
            None,
            None
        ).unwrap();
        
        // Generate all numbers and check if 16477014383 is present
        let mut found = false;
        let target = "16466513583";
        
        while let Some(number) = gen.next() {
            if number == target {
                found = true;
                break;
            }
        }
        
        assert!(found, "Expected number {} was not generated", target);
        println!("Successfully generated target US number: {}", target);
    }
    
    #[test]
    fn test_specific_number_generation_sg() {
        // Test case 2: Ensure SG number 6583554902 is generated with prefix 835 and suffix 2
        let _ = load_format_data();
        let sg_format = get_country_format("sg").unwrap();
        
        // Create generator for SG with prefix and suffix
        let mut gen = PhoneNumberGenerator::new(
            &sg_format,
            Some("835".to_string()),  // Prefix
            Some("2".to_string()),    // Suffix
            None,
            None
        ).unwrap();
        
        // Generate all numbers and check if 6583554902 is present
        let mut found = false;
        let target = "6583554902";
        
        while let Some(number) = gen.next() {
            if number == target {
                found = true;
                break;
            }
        }
        
        assert!(found, "Expected number {} was not generated", target);
        println!("Successfully generated target Singapore number: {}", target);
    }
    
    #[test]
    fn test_specific_number_generation_jp() {
        // Test case 3: Ensure Japan number 819012345678 is generated with prefix 90 and suffix 78
        let _ = load_format_data();
        let jp_format = get_country_format("jp").unwrap();
        
        // Create generator for Japan with prefix and suffix
        let mut gen = PhoneNumberGenerator::new(
            &jp_format,
            Some("90".to_string()),  // Prefix (mobile phone)
            Some("78".to_string()),  // Suffix
            None,
            None
        ).unwrap();
        
        // Generate all numbers and check if 819012345678 is present
        let mut found = false;
        let target = "819012345678";
        
        while let Some(number) = gen.next() {
            if number == target {
                found = true;
                break;
            }
        }
        
        assert!(found, "Expected number {} was not generated", target);
        println!("Successfully generated target Japan number: {}", target);
    }

    #[test]
    fn test_number_generation_with_prefix() {
        // Load format data
        let _ = load_format_data();
        
        // Get US format
        let us_format = get_country_format("us").unwrap();
        
        // Test with area code prefix
        let mut gen_with_prefix = PhoneNumberGenerator::new(
            &us_format, 
            Some("218".to_string()), // Use area code 218 as prefix
            None, 
            None,
            None
        ).unwrap();
        
        // Get first number and verify format
        let first_number = gen_with_prefix.next().unwrap();
        
        // Expected format: 1 (country code) + 218 (prefix) + 7 digits = 11 digits total
        assert!(first_number.starts_with("1218"), "Number with prefix should start with country code + prefix");
        
        // Test with larger prefix
        let mut gen_with_longer_prefix = PhoneNumberGenerator::new(
            &us_format, 
            Some("218555".to_string()), // Use longer prefix
            None, 
            None,
            None
        ).unwrap();
        
        let first_longer_prefix = gen_with_longer_prefix.next().unwrap();
        assert!(first_longer_prefix.starts_with("1218555"), "Number with longer prefix should be formatted correctly");
        
        // Test with suffix filter
        let mut gen_with_suffix = PhoneNumberGenerator::new(
            &us_format, 
            Some("218".to_string()),
            Some("19".to_string()), // Only want numbers ending with 19
            None,
            None
        ).unwrap();
        
        let suffix_number = gen_with_suffix.next().unwrap();
        assert!(suffix_number.starts_with("1218"), "Number should start with country code + prefix");
        assert!(suffix_number.ends_with("19"), "Number should end with specified suffix");
    }
    
    #[test]
    fn test_number_generation_singapore() {
        // Load format data
        let _ = load_format_data();
        
        // Get Singapore format
        let sg_format = get_country_format("sg").unwrap();
        
        // Test with area code 8
        let mut gen_without_prefix = PhoneNumberGenerator::new(
            &sg_format, 
            None,  // No prefix, use area codes from format
            None,  // No suffix filter
            None,
            None   // Use default digits from format
        ).unwrap();
        
        // Get first number and verify format
        let first_number = gen_without_prefix.next().unwrap();
        
        // Should have country code (65) + area code (8 or 9) + remaining digits
        assert!(first_number.starts_with("658") || first_number.starts_with("659"), 
                "Singapore number should start with 65 + area code");
        
        // Test with specific prefix (e.g., 91 for certain mobile numbers)
        let mut gen_with_prefix = PhoneNumberGenerator::new(
            &sg_format, 
            Some("91".to_string()),  // Prefix 91 (common mobile prefix)
            None,
            None,
            None
        ).unwrap();
        
        // Get first number with prefix and verify format
        let prefix_number = gen_with_prefix.next().unwrap();
        assert!(prefix_number.starts_with("6591"), "Singapore number with prefix should start with 6591");
        
        // Test with suffix filter
        let mut gen_with_suffix = PhoneNumberGenerator::new(
            &sg_format, 
            Some("91".to_string()),  // Prefix 91
            Some("99".to_string()),  // Should end with 99
            None,
            None
        ).unwrap();
        
        // Get number with suffix filter and verify
        let suffix_number = gen_with_suffix.next().unwrap();
        assert!(suffix_number.starts_with("6591"), "Singapore number should start with 6591");
        assert!(suffix_number.ends_with("99"), "Singapore number should end with 99");
    }

    #[test]
    fn test_infix_filtering() {
        // Load format data
        let _ = load_format_data();
        
        // Get US format
        let us_format = get_country_format("us").unwrap();
        
        // Test with infix filter
        let mut gen_with_infix = PhoneNumberGenerator::new(
            &us_format, 
            Some("212".to_string()), // New York area code
            None,                    // No suffix
            Some("02".to_string()),  // Infix filter: 02
            None                     // Default digits
        ).unwrap();
        
        // Generate some numbers and check if they all have the specified infix
        for _ in 0..10 {
            if let Some(number) = gen_with_infix.next() {
                let len = number.len();
                if len >= 6 {
                    let extracted_infix = &number[len - 6..len - 4];
                    assert_eq!(extracted_infix, "02", "Generated number should have infix '02'");
                } else {
                    panic!("Generated number is too short for infix: {}", number);
                }
            } else {
                // It's possible we run out of numbers with this specific infix
                break;
            }
        }
        
        // Test with both infix and suffix
        let mut gen_with_both = PhoneNumberGenerator::new(
            &us_format,
            Some("312".to_string()),   // Chicago area code
            Some("99".to_string()),    // Suffix: 99
            Some("45".to_string()),    // Infix: 45
            None                       // Default digits
        ).unwrap();
        
        // Generate some numbers and check if they have both the infix and suffix
        for _ in 0..5 {
            if let Some(number) = gen_with_both.next() {
                let len = number.len();
                // Check suffix
                assert!(number.ends_with("99"), "Number should end with '99': {}", number);
                // Check infix
                if len >= 6 {
                    let extracted_infix = &number[len - 6..len - 4];
                    assert_eq!(extracted_infix, "45", "Generated number should have infix '45'");
                } else {
                    panic!("Generated number is too short for infix: {}", number);
                }
            } else {
                // It's possible we run out of numbers with this specific combination
                break;
            }
        }
        
        // Test estimate adjustments
        let gen_no_filter = PhoneNumberGenerator::new(
            &us_format,
            Some("202".to_string()),  // DC area code
            None,                     // No suffix
            None,                     // No infix
            None                      // Default digits
        ).unwrap();
        
        let gen_with_infix_filter = PhoneNumberGenerator::new(
            &us_format,
            Some("202".to_string()),  // DC area code
            None,                     // No suffix
            Some("33".to_string()),   // Infix: 33
            None                      // Default digits
        ).unwrap();
        
        // The estimate with infix filter should be approximately 1/100 of the estimate without filter
        let estimate_no_filter = gen_no_filter.estimate_total();
        let estimate_with_infix = gen_with_infix_filter.estimate_total();
        
        // Allow some margin for rounding
        assert!(estimate_with_infix * 90 <= estimate_no_filter && estimate_with_infix * 110 >= estimate_no_filter,
                "Infix estimate should be approximately 1/100 of regular estimate");
    }
}