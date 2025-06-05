use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use anyhow::{Error, Result, anyhow};
use crate::cli::Mode;
use crate::format::{get_country_format, PhoneNumberGenerator};

// Modify the calculate_estimate function to include infix parameter
pub async fn calculate_estimate(
    mode: Mode,
    input_file_path: &str,
    prefix: &str,
    suffix: &str,
    infix: Option<&str>,
    digits: Option<usize>
) -> Result<u64, Error> {
    match mode {
        Mode::Quick => estimate_file_workload(input_file_path, prefix, suffix, infix).await,
        Mode::Full => estimate_fullscan_workload(input_file_path, prefix, suffix, infix, digits),
        _ => Ok(100) // Minimal estimate for other modes
    }
}

/// Estimate workload for quick scan (file) mode
async fn estimate_file_workload(
    input_file_path: &str, 
    prefix: &str, 
    suffix: &str,
    infix: Option<&str>
) -> Result<u64, Error> {
    // Check if file exists
    if !Path::new(input_file_path).exists() {
        return Err(anyhow!("File not found: {}", input_file_path));
    }
    
    // Get file size for estimation
    let metadata = tokio::fs::metadata(input_file_path).await?;
    let file_size = metadata.len();
    
    // Check if file is empty
    if file_size == 0 {
        return Err(anyhow!("File is empty: {}", input_file_path));
    }
    
    // Sample a portion of the file for better accuracy
    const SAMPLE_SIZE: u64 = 50_000;
    let (sample_matched, sample_count, sample_bytes_read) = sample_file(
        input_file_path, 
        SAMPLE_SIZE, 
        prefix, 
        suffix,
        infix
    ).await?;
    
    // Handle the case where no valid lines were found
    if sample_count == 0 {
        return Err(anyhow!("No valid phone numbers found in file: {}", input_file_path));
    }
    
    // Calculate total estimate
    let match_ratio = sample_matched as f64 / sample_count as f64;
    let bytes_per_line = if sample_count > 0 { 
        sample_bytes_read as f64 / sample_count as f64 
    } else { 
        50.0 // Default estimate
    };
    
    let estimated_total_lines = (file_size as f64 / bytes_per_line).ceil() as u64;
    let estimated_matches = (estimated_total_lines as f64 * match_ratio).ceil() as u64;
    
    // Add a buffer to ensure we don't underestimate (increase by 10%)
    let estimated_matches = (estimated_matches as f64 * 1.1).ceil() as u64;
    
    // Ensure a reasonable minimum
    let estimated_matches = std::cmp::max(estimated_matches, 1000);
    
    Ok(estimated_matches)
}

/// Sample a file to get an estimate of matching lines
async fn sample_file(
    input_file_path: &str, 
    sample_size: u64,
    prefix: &str,
    suffix: &str,
    infix: Option<&str>
) -> Result<(u64, u64, u64), Error> {
    let sample_file = File::open(input_file_path).await?;
    let sample_reader = BufReader::new(sample_file);
    let mut sample_lines = sample_reader.lines();
    
    let mut sample_count = 0;
    let mut sample_matched = 0;
    let mut sample_bytes_read = 0;
    let check_suffix = !suffix.is_empty();
    let check_prefix = !prefix.is_empty();
    let check_infix = infix.is_some();
    
    // Read the sample
    while let Some(line) = sample_lines.next_line().await? {
        if sample_count >= sample_size {
            break;
        }
        
        sample_bytes_read += line.len() as u64 + 1; // +1 for newline
        
        if !line.trim().is_empty() {
            sample_count += 1;
            
            let phone = line.trim();
            let suffix_match = !check_suffix || phone.ends_with(suffix);
            let prefix_match = !check_prefix || phone.starts_with(prefix);
            
            // Check infix if needed
            let infix_match = if check_infix {
                let infix_val = infix.unwrap();
                if phone.len() >= 6 {
                    // Extract the infix (6th and 5th characters from the end)
                    let potential_infix = &phone[phone.len() - 6..phone.len() - 4];
                    potential_infix == infix_val
                } else {
                    false // Phone number too short for infix
                }
            } else {
                true // No infix check needed
            };
            
            if suffix_match && prefix_match && infix_match {
                sample_matched += 1;
            }
        }
    }
    
    Ok((sample_matched, sample_count, sample_bytes_read))
}

/// Estimate workload for full scan (number generation) mode
fn estimate_fullscan_workload(
    country_code: &str,
    prefix: &str,
    suffix: &str,
    infix: Option<&str>,
    digits_override: Option<usize>
) -> Result<u64, Error> {
    // Try to get country format
    match get_country_format(country_code) {
        Ok(format) => {
            // Create a generator to get estimate
            let generator = PhoneNumberGenerator::new(
                &format,
                if prefix.is_empty() { None } else { Some(prefix.to_string()) },
                if suffix.is_empty() { None } else { Some(suffix.to_string()) },
                infix.map(|s| s.to_string()),
                digits_override
            )?;
            
            Ok(generator.estimate_total())
        },
        Err(_) => {
            // If no format, use digits parameter
            if let Some(digit_count) = digits_override {
                let total = 10_u64.pow(digit_count as u32);
                
                // Apply reduction for infix if present
                let adjusted_total = if let Some(infix_val) = infix {
                    // Each digit in the infix reduces the probability by a factor of 10
                    let infix_factor = 10_u64.pow(infix_val.len() as u32);
                    std::cmp::max(1, total / infix_factor)
                } else {
                    total
                };
                
                Ok(adjusted_total)
            } else {
                Err(anyhow!("Cannot estimate: No country format found and no digits override provided"))
            }
        }
    }
}