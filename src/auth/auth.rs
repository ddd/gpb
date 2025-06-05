use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use anyhow::{Result, Error, anyhow};
use lazy_static::lazy_static;
use regex::Regex;

#[cfg(test)]
use tokio::runtime::Runtime;

// Structure to store authentication credentials with expiration
pub struct AuthCredentials {
    cookie: String,
    gxf: String,
    azt: String,
    ist: String,
    last_refresh: Instant,
    valid_for: Duration,
}

impl AuthCredentials {
    fn new() -> Self {
        Self {
            cookie: String::new(),
            gxf: String::new(),
            azt: String::new(),
            ist: String::new(),
            last_refresh: Instant::now(),
            valid_for: Duration::from_secs(12 * 60 * 60), // 12 hours validity
        }
    }

    fn is_valid(&self) -> bool {
        !self.cookie.is_empty() && 
        !self.gxf.is_empty() && 
        self.last_refresh.elapsed() < self.valid_for
    }
}

// Global auth credentials storage
lazy_static! {
    static ref AUTH_CREDENTIALS: Arc<RwLock<AuthCredentials>> = Arc::new(RwLock::new(AuthCredentials::new()));
}

// Extract cookie from response headers
fn extract_cookie(headers: &reqwest::header::HeaderMap) -> Result<String, Error> {
    for (name, value) in headers.iter() {
        if name == "set-cookie" {
            let value_str = value.to_str()?;
            if value_str.contains("__Host-GAPS") {
                // Extract the __Host-GAPS cookie
                let cookie_parts: Vec<&str> = value_str.split(';').collect();
                return Ok(cookie_parts[0].to_string());
            }
        }
    }
    Err(anyhow!("Cookie not found in response"))
}

// Extract GXF token from no-js page (XSRF)
fn extract_gxf(html_content: &str) -> Result<String, Error> {
    // Regular expression to find the GXF token
    let re = Regex::new(r#"id="gxf" value="([_a-zA-Z].+:\d+)">"#)?;
    
    if let Some(captures) = re.captures(html_content) {
        if let Some(token_match) = captures.get(1) {
            return Ok(token_match.as_str().to_string());
        }
    }
    
    Err(anyhow!("GXF token not found in HTML content"))
}

// Extract AZT token from js page (XSRF)
fn extract_azt(html_content: &str) -> Result<String, Error> {
    // Regular expression to find the AZT token
    let re = Regex::new(r#"\\"xsrf\\",null,\[\\"\\"\],\\"([_a-zA-Z].+:\d+)\\"]","Qzxixc""#)?;
    
    if let Some(captures) = re.captures(html_content) {
        if let Some(token_match) = captures.get(1) {
            return Ok(token_match.as_str().to_string());
        }
    }
    
    Err(anyhow!("AZT token not found in HTML content"))
}

// Extract IST (initial session token) from js page (XSRF)
fn extract_ist(html_content: &str) -> Result<String, Error> {
    // Regular expression to find the AZT token
    let re = Regex::new(r#"data-initial-setup-data="%.@.null,null,null,null,null,null,null,null,null,&quot;..&quot;,null,null,null,&quot;([a-zA-Z0-9-_]*)&quot;"#)?;
    
    if let Some(captures) = re.captures(html_content) {
        if let Some(token_match) = captures.get(1) {
            return Ok(token_match.as_str().to_string());
        }
    }
    
    Err(anyhow!("IST token not found in HTML content"))
}

/// Fetch fresh authentication credentials
async fn fetch_auth_credentials() -> Result<(String, String, String, String), Error> {
    let client = crate::utils::create_client(None, "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:87.0) Gecko/20100101 Cobalt/87.0");
    
    // no-js page
    let response = client
        .get("https://accounts.google.com/signin/usernamerecovery?hl=en")
        .send()
        .await?;
    
    // Extract cookie from headers
    let cookie = extract_cookie(response.headers())?;
    
    // Get HTML content
    let html_content = response.text().await?;
    
    // Extract GXF token from HTML
    let gxf = extract_gxf(&html_content)?;

    let client = crate::utils::create_client(None, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36");

    // js page
    let response = client
        .get("https://accounts.google.com/signin/v2/usernamerecovery?hl=en")
        .send()
        .await?;

    // Get HTML content
    let html_content = response.text().await?;

    let azt = extract_azt(&html_content)?;

    let ist = extract_ist(&html_content)?;

    Ok((cookie, gxf, azt, ist))
}

/// Get current authentication credentials, refreshing if needed
pub async fn get_auth_credentials() -> Result<(String, String, String, String), Error> {
    // Check if we already have valid credentials
    {
        let auth_read = AUTH_CREDENTIALS.read().unwrap();
        if auth_read.is_valid() {
            return Ok((auth_read.cookie.clone(), auth_read.gxf.clone(), auth_read.azt.clone(), auth_read.ist.clone()));
        }
    }
    
    // If not valid, fetch new credentials
    let (cookie, gxf, azt, ist) = fetch_auth_credentials().await?;
    
    // Update stored credentials
    {
        let mut auth_write = AUTH_CREDENTIALS.write().unwrap();
        auth_write.cookie = cookie.clone();
        auth_write.gxf = gxf.clone();
        auth_write.azt = azt.clone();
        auth_write.ist = ist.clone();
        auth_write.last_refresh = Instant::now();
    }
    
    Ok((cookie, gxf, azt, ist))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_credentials_fetch() {
        // Create a tokio runtime for async testing
        let rt = Runtime::new().unwrap();
        
        // Use the runtime to run our async test
        rt.block_on(async {
            // Test fetching credentials
            let result = fetch_auth_credentials().await;
            
            match result {
                Ok((cookie, gxf, azt, ist)) => {
                    // Check that we got valid credentials
                    assert!(cookie.contains("__Host-GAPS"), "Cookie should contain __Host-GAPS");
                    
                    println!("Successfully retrieved credentials:");
                    println!("Cookie: {}", cookie);
                    println!("GXF: {}", gxf);
                    println!("AZT: {}", azt);
                    println!("IST: {}", ist);
                },
                Err(e) => {
                    panic!("Failed to fetch auth credentials: {}", e);
                }
            }
        });
    }
    
    #[test]
    fn test_regex_extraction() {
        // Test cookie extraction
        let sample_header = "Set-Cookie: __Host-GAPS=1:rd1j05ucgjm9dgQKxu3oYroqXB5Idw:UrVHk20n2GqaCKhd;Path=/;Expires=Fri, 26-Mar-2027 03:54:09 GMT;Secure;HttpOnly;Priority=HIGH";
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("set-cookie", sample_header.parse().unwrap());
        
        let cookie_result = extract_cookie(&headers);
        assert!(cookie_result.is_ok(), "Should extract cookie from header");
        
        // Test GXF extraction
        let sample_html = r#"<input name="hl" type="hidden" value="en"><input type="hidden" name="gxf" id="gxf" value="AFoagUWcY46prQ4R_INgj3mIaEuBkOaWpg:1743058617372"><input type="hidden" id="_utf8" name="_utf8" value="&#9731;">
        
        
        
        "#;
        
        let gxf_result = extract_gxf(sample_html);
        assert!(gxf_result.is_ok(), "Should extract GXF from HTML");
    }
}