use anyhow::{anyhow, Error, Result};
use reqwest::Client;
use crate::auth;
use crate::botguard;
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct AccountLookupResponse {
    #[prost(enumeration = "account_lookup_response::Status", tag = "1")]
    pub status: i32,
}

// Define the enum values for Status
pub mod account_lookup_response {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Status {
        UnknownStatus = 0,
        InvalidIdentifier = 2,
        Captcha = 5,
        Found = 6,
        NotFound = 7,
    }
    // 2 is unknown identifier
}

/// Performs a lookup to check if a phone number or email exists in Google's system
pub async fn lookup(client: &Client, identifier: &str, first_name: &str, last_name: &str) -> Result<bool, Error> {
    // Get authentication credentials
    let (cookie, _, azt, ist) = auth::get_auth_credentials().await?;

    // Get a valid botguard token for this lookup. Name does not matter for the no-js endpoint.
    let bg_token = botguard::wait_for_valid_token(true, first_name, last_name).await?;

    // Encode the identifier for the request
    let encoded_identifier = urlencoding::encode(identifier);
    
    // Request
    let request = client
        .post("https://accounts.google.com/_/lookup/accountlookup?hl=en&rt=b")
        .header("Content-Type", "application/x-www-form-urlencoded;charset=UTF-8")
        .header("Cookie", &cookie)
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Google-Accounts-Xsrf", "1")
        .body(format!("hl=en&ddm=1&continue=https%3A%2F%2Faccounts.google.com%2FManageAccount%3Fnc%3D1&f.req=%5B%22{}%22%2C%22{}%22%2Cnull%2Cnull%2Cnull%2C%22{}%22%2C%22{}%22%2C1%2C0%2Cnull%2C%5Bnull%2Cnull%2C%5B2%2C1%2Cnull%2Cnull%2C%22https%3A%2F%2Faccounts.google.com%2FServiceLogin%3Fhl%3Den%22%2Cnull%2Cnull%2C5%2Cnull%2C%22GlifWebSignIn%22%2Cnull%2Cnull%2C1%5D%2C1%2C%5B%5D%2Cnull%2Cnull%2Cnull%2C1%2Cnull%2Cnull%2Cnull%2Cnull%2Cnull%2Cnull%2Cnull%2Cnull%2C%5B%5D%2Cnull%2Cnull%2C3%5D%5D&bgRequest=%5B%22username-recovery%22%2C%22{}%22%5D&azt={}&cookiesDisabled=false&gmscoreversion=undefined&flowName=GlifWebSignIn&checkConnection=youtube%3A591&checkedDomains=youtube&pstMsg=1&", encoded_identifier, ist, first_name, last_name, bg_token, azt));

    let response = request
        .send()
        .await?;

    if response.status() != 200 {
        return Err(anyhow!("unexpected status code in first request: {}", response.status()));
    }

    // Get the response bytes directly for protobuf decoding
    let response_bytes = response.bytes().await?;

    // Decode the protobuf response
    return match AccountLookupResponse::decode(&response_bytes[..]) {
        Ok(response) => {
            // Check the status
            match response.status {
                6 => Ok(true),  // Status::Found = 6
                7 => Ok(false), // Status::NotFound = 7
                5 => Err(anyhow!("ratelimited")), // Status::Captcha = 5
                2 => Err(anyhow!("invalid_identifier")),  // Status::InvalidIdentifier, this happens on some phone formats as our format.json may not be 100% accurate and pass libphonenumber validation
                _ => Err(anyhow!("Unknown response status: {}", response.status)),
            }
        },
        Err(e) => {
            Err(anyhow!("Failed to decode protobuf response: {}", e))
        }
    };
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