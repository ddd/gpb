use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use anyhow::{Result, Error, anyhow};
use reqwest::Client;
use serde::Deserialize;
use tokio::time::sleep;
use lazy_static::lazy_static;

// Structure to store BG token with associated metadata
#[derive(Clone, Debug)]
pub struct BotguardToken {
    pub token: String,
    pub first_name: String,
    pub last_name: String,
    pub created_at: Instant,
    pub static_token: bool,
}

impl BotguardToken {
    fn new(token: String, first_name: String, last_name: String, static_token: bool) -> Self {
        Self {
            token,
            first_name,
            last_name,
            created_at: Instant::now(),
            static_token,
        }
    }

    // Check if token is still considered valid
    fn is_valid(&self) -> bool {
        // Static tokens are always considered valid
        if self.static_token {
            return true;
        }
        self.created_at.elapsed() < Duration::from_secs(30 * 60) // 9 minutes (slightly less than refresh period)
    }
}

// Response structure from the API
#[derive(Deserialize, Debug)]
struct BotguardResponse {
    bgToken: String,
}

// Error response from the API
#[derive(Deserialize, Debug)]
struct BotguardErrorResponse {
    error: String,
}

// Global storage for the botguard token
lazy_static! {
    static ref BOTGUARD_TOKEN: Arc<RwLock<Option<BotguardToken>>> = Arc::new(RwLock::new(None));
    static ref REQUESTED_NAMES: Arc<RwLock<(String, String)>> = Arc::new(RwLock::new((String::new(), String::new())));
}

/// Set the requested first name for the next botguard token
pub fn set_bg_firstname(first_name: &str) {
    let mut names = REQUESTED_NAMES.write().unwrap();
    names.0 = first_name.to_string();
}

/// Set the requested last name for the next botguard token
pub fn set_bg_lastname(last_name: &str) {
    let mut names = REQUESTED_NAMES.write().unwrap();
    names.1 = last_name.to_string();
}

/// Set a static botguard token that won't be refreshed
pub fn set_static_bg_token(token: &str) {
    let names = {
        let names_read = REQUESTED_NAMES.read().unwrap();
        names_read.clone()
    };
    
    // Update the global token with the static token
    let mut token_write = BOTGUARD_TOKEN.write().unwrap();
    *token_write = Some(BotguardToken::new(
        token.to_string(),
        names.0.clone(),
        names.1.clone(),
        true, // Mark as static token
    ));
    
    println!("Using static botguard token (will not be refreshed)");
}

/// Check if we're using a static botguard token
pub fn is_using_static_token() -> bool {
    let token_read = BOTGUARD_TOKEN.read().unwrap();
    if let Some(token) = &*token_read {
        token.static_token
    } else {
        false
    }
}

/// Get the current botguard token
pub fn get_bg_token() -> Option<(String, String, String)> {
    let token_read = BOTGUARD_TOKEN.read().unwrap();
    
    if let Some(token) = &*token_read {
        if token.is_valid() {
            return Some((token.first_name.clone(), token.last_name.clone(), token.token.clone()));
        }
    }
    
    None
}

/// Force an immediate update of the botguard token
pub async fn force_bg_update() -> Result<(), Error> {
    // Check if we're using a static token
    if is_using_static_token() {
        return Ok(());  // Don't update if using static token
    }
    
    // Get the requested names
    let names = {
        let names_read = REQUESTED_NAMES.read().unwrap();
        names_read.clone()
    };

    // Fetch new token
    match fetch_bg_token(&names.0, &names.1).await {
        Ok(token) => {
            // Update the global token
            let mut token_write = BOTGUARD_TOKEN.write().unwrap();
            *token_write = Some(BotguardToken::new(
                token,
                names.0.clone(),
                names.1.clone(),
                false, // Not a static token
            ));
            Ok(())
        },
        Err(e) => Err(e),
    }
}

/// Check if the local botguard token generation server is running
pub async fn ping_botguard_server() -> bool {
    let client = reqwest::Client::new();
    match client.get("http://localhost:7912/api/ping")
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await {
            Ok(response) => {
                if let Ok(text) = response.text().await {
                    text.trim() == "pong"
                } else {
                    false
                }
            },
            Err(_) => false
        }
}

/// Fetch a new botguard token from the API
async fn fetch_bg_token(first_name: &str, last_name: &str) -> Result<String, Error> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    
    // Build the URL with query parameters
    let mut url = String::from("http://localhost:7912/api/generate_bgtoken");
    
    if !first_name.is_empty() || !last_name.is_empty() {
        url.push('?');
        
        if !first_name.is_empty() {
            url.push_str(&format!("firstName={}", urlencoding::encode(first_name)));
        }
        
        if !first_name.is_empty() && !last_name.is_empty() {
            url.push('&');
        }
        
        if !last_name.is_empty() {
            url.push_str(&format!("lastName={}", urlencoding::encode(last_name)));
        }
    }
    
    // Make the request
    let response = client.get(&url).send().await?;
    
    // Check if request was successful
    if response.status().is_success() {
        let bg_response = response.json::<BotguardResponse>().await?;
        Ok(bg_response.bgToken)
    } else {
        Err(anyhow!("Botguard API error: HTTP {}", response.status()))
    }
}

/// Wait until we have a valid token that matches the requested names
pub async fn wait_for_valid_token(require_name_match: bool, first_name: &str, last_name: &str) -> Result<String, Error> {
    // First check if we're using a static token
    if is_using_static_token() {
        // For static tokens, just return the token without checking names
        if let Some((_, _, token)) = get_bg_token() {
            return Ok(token);
        }
    }

    let max_attempts = 60; // 30 seconds max (500ms * 60)
    let mut attempts = 0;
    
    loop {
        // Get the current token
        if let Some((token_first, token_last, token)) = get_bg_token() {
            // Check if the token matches the requested names
            if !require_name_match || (token_first == first_name && token_last == last_name) {
                return Ok(token);
            }
        }
        
        // Increment attempts and check if we've reached the max
        attempts += 1;
        if attempts >= max_attempts {
            return Err(anyhow!("Failed to get valid botguard token after {} attempts", max_attempts));
        }
        
        // Wait before retrying
        sleep(Duration::from_millis(500)).await;
    }
}

/// Start a background task that periodically refreshes the botguard token
pub async fn start_bg_token_refresh_task() {
    // Check if we're using a static token
    if is_using_static_token() {
        return;  // Don't start refresh task for static tokens
    }

    let refresh_interval = Duration::from_secs(10 * 60); // 10 minutes
    
    tokio::spawn(async move {
        loop {
            // Sleep first - we assume an initial token has been fetched
            sleep(refresh_interval).await;
            
            // Check again if a static token was set while we were sleeping
            if is_using_static_token() {
                println!("Static token detected - terminating background refresh task");
                break;
            }
            
            if let Err(e) = force_bg_update().await {
                eprintln!("Error refreshing botguard token: {}", e);
            } else {
                //println!("Botguard token refreshed successfully");
            }
        }
    });
}

#[cfg(test)]
use tokio::runtime::Runtime;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fetch_botguard_token() {
        // Create a tokio runtime for async testing
        let rt = Runtime::new().unwrap();
        
        // Use the runtime to run our async test
        rt.block_on(async {
            // Test with custom names
            let first_name = "JohnTest";
            let last_name = "DoeTest";
            
            // Set the names
            set_bg_firstname(first_name);
            set_bg_lastname(last_name);
            
            // Fetch a token
            println!("Fetching botguard token from API...");
            let result = fetch_bg_token(first_name, last_name).await;
            
            match result {
                Ok(token) => {
                    // Check that we got a non-empty token
                    assert!(!token.is_empty(), "Token should not be empty");
                    
                    println!("Successfully retrieved botguard token:");
                    println!("Token (first 50 chars): {}", &token[0..50.min(token.len())]);
                    println!("Token length: {} characters", token.len());
                },
                Err(e) => {
                    panic!("Failed to fetch botguard token: {}", e);
                }
            }
            
            // Test the force update function
            let update_result = force_bg_update().await;
            assert!(update_result.is_ok(), "force_bg_update() should succeed");
            
            // Verify we can get the token now
            let token_option = get_bg_token();
            assert!(token_option.is_some(), "get_bg_token() should return Some after force_bg_update()");
            
            if let Some((token_first, token_last, token)) = token_option {
                assert_eq!(token_first, first_name, "First name should match what we set");
                assert_eq!(token_last, last_name, "Last name should match what we set");
                assert!(!token.is_empty(), "Token should not be empty");
            }
            
            // Test setting a static token
            set_static_bg_token("test_static_token");
            
            // Verify the static token is returned
            let static_token_option = get_bg_token();
            assert!(static_token_option.is_some(), "get_bg_token() should return Some after set_static_bg_token()");
            
            if let Some((_, _, token)) = static_token_option {
                assert_eq!(token, "test_static_token", "Token should match what we set");
                assert!(is_using_static_token(), "is_using_static_token() should return true");
            }
        });
    }
}