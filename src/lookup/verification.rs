use anyhow::{Error, Result};
use reqwest::Client;
use crate::lookup::nojs;
use crate::models::{FAKE_FIRST_NAME, FAKE_LAST_NAME};

/// Verifies if a hit is real or fake by testing with various name combinations
/// For phone numbers, this helps filter out false positives
pub async fn verify_hit(client: &Client, identifier: &str, first_name: &str, last_name: &str) -> Result<bool, Error> {

    // Check 1: Try with completely fake names
    let fake_result = nojs::lookup(client, identifier, FAKE_FIRST_NAME, FAKE_LAST_NAME).await?;
    
    // If it's a hit with fake names, then it's a fake hit
    if fake_result {
        return Ok(false); // Not a real hit
    }
    
    // Check 2: Skip the second check if last_name is empty
    if !last_name.is_empty() {
        // Try with real first name but fake last name
        // This checks for the case where an account only has a first name
        let first_name_only_result = nojs::lookup(client, identifier, first_name, FAKE_LAST_NAME).await?;
        
        // If we get a hit with real first name + fake last name, it's likely matching only on first name
        if first_name_only_result {
            return Ok(false); // Not a real hit - likely only has first name, no last name
        }
    }
    
    // If we've passed all applicable checks, it's likely a real hit
    Ok(true)
}