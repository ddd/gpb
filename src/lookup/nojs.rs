use anyhow::{anyhow, Error, Result};
use reqwest::Client;
use crate::auth;
use crate::botguard;

/// Performs a lookup to check if a phone number or email exists in Google's system
pub async fn lookup(client: &Client, identifier: &str, first_name: &str, last_name: &str) -> Result<bool, Error> {
    // Get authentication credentials
    let (cookie, gxf, _, _) = auth::get_auth_credentials().await?;

    // Get a valid botguard token for this lookup. Name does not matter for the no-js endpoint.
    let bg_token = botguard::wait_for_valid_token(false, first_name, last_name).await?;
    
    // First request
    let first_request = client
        .post("https://accounts.google.com/signin/usernamerecovery")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Cookie", &cookie)
        .body(format!("Email={}&gxf={}", urlencoding::encode(identifier), gxf));

    let first_response = first_request
        .send()
        .await?;

    // invalid identifier
    if first_response.status() == 200 {
        return Ok(false);
    }

    if !first_response.status().is_redirection() {
        let status_code = first_response.status();
        let body = first_response.text().await?;
        return Err(anyhow!("unexpected status code in first request: {}: {}", status_code, body));
    }

    // Get the location header and extract the ess parameter
    let location = match first_response.headers().get("location") {
        Some(loc) => loc.to_str()?,
        None => return Err(anyhow!("no location header in first response")),
    };

    // Extract the ess parameter from the Location URL
    let ess = match location.split("ess=").nth(1) {
        Some(s) => s,
        None => return Err(anyhow!("no ess parameter in location header")),
    };
    
    // Second request - using the dynamically obtained botguard token
    let second_request = client
        .post("https://accounts.google.com/signin/usernamerecovery/lookup")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Cookie", &cookie)
        .body(format!(
            "challengeId=0&challengeType=28&hl=en-GB&ess={}&gxf={}&bgresponse={}&GivenName={}&FamilyName={}",
            ess, gxf, bg_token, urlencoding::encode(first_name), urlencoding::encode(last_name)
        ));

    let second_response = second_request
        .send()
        .await?;
    
    // If it's status code 200, it failed so we need to retry
    if second_response.status().as_u16() == 200 {
        return Err(anyhow!("ratelimited")); // Return ratelimited error to trigger retry
    }
    
    if second_response.status().is_redirection() {
        let location = match second_response.headers().get("location") {
            Some(loc) => loc.to_str()?,
            None => return Err(anyhow!("no location header in second response")),
        };
        
        if location.contains("/signin/usernamerecovery/challenge") {
            return Ok(true);
        } else if location.contains("/signin/usernamerecovery/noaccountsfound") {
            return Ok(false);
        } else if location.contains("/signin/rejected?rrk=54") {
            // botguard token expired
            return Err(anyhow!("botguard token expired"));
        }
    }
    
    // If we get here, it's something unexpected
    Err(anyhow!("unexpected response in second request: status {}", second_response.status()))
}

#[cfg(test)]
use crate::utils::create_client;

#[tokio::test]
async fn test_lookup_valid_hit() {
    let client = create_client(None, "");
    let valid_phone = "31658854003";
    let first_name = "Henry";
    let last_name = "Chancellor";
    
    // Set up the botguard token with correct names
    botguard::set_bg_firstname(first_name);
    botguard::set_bg_lastname(last_name);
    match botguard::force_bg_update().await {
        Ok(_) => {}
        Err(e) => {
            println!("Warning: Failed to update botguard token: {}", e);
        }
    }
    
    println!("Testing valid phone number: {}", valid_phone);
    let result = lookup(&client, valid_phone, first_name, last_name).await;
    
    match result {
        Ok(exists) => {
            assert!(exists, "Expected phone {} to be a hit", valid_phone);
            println!("Test PASSED: Valid phone number correctly identified as a hit");
        },
        Err(e) => {
            panic!("Test FAILED: Error during lookup for valid phone: {}", e);
        }
    }
}

#[tokio::test]
async fn test_lookup_invalid_hit() {
    let client = create_client(None, "");
    let invalid_phone = "31644854003";
    let first_name = "Henry";
    let last_name = "Chancellor";
    
    // Set up the botguard token with correct names
    botguard::set_bg_firstname(first_name);
    botguard::set_bg_lastname(last_name);
    match botguard::force_bg_update().await {
        Ok(_) => {}
        Err(e) => {
            println!("Warning: Failed to update botguard token: {}", e);
        }
    }
    
    println!("Testing invalid phone number: {}", invalid_phone);
    let result = lookup(&client, invalid_phone, first_name, last_name).await;
    
    match result {
        Ok(exists) => {
            assert!(!exists, "Expected phone {} to NOT be a hit", invalid_phone);
            println!("Test PASSED: Invalid phone number correctly identified as not a hit");
        },
        Err(e) => {
            panic!("Test FAILED: Error during lookup for invalid phone: {}", e);
        }
    }
}