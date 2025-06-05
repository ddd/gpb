use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::sleep;
use tracing::error;

use crate::models::Counters;
use crate::auth;
use crate::lookup::{js, nojs};
use crate::cli::LookupType;

// Message types for worker communication
pub enum WorkerMessage {
    // A phone number to check, with record id and identifier
    CheckPhone {
        record_id: usize,
        phone: String,
        identifier: String,     // Added identifier field
        first_name: String,
        last_name: String,
        pending_counter: Option<Arc<std::sync::atomic::AtomicUsize>>, // Counter to track pending tasks
    },
    // Signal workers to shut down
    Shutdown,
}

// Message types for result communication
pub enum ResultMessage {
    // A hit was found for a record
    Hit {
        record_id: usize,
        phone: String,
    },
}

// CSV Worker function that processes phone numbers from the queue
pub async fn csv_worker(
    work_rx: async_channel::Receiver<WorkerMessage>,
    result_tx: async_channel::Sender<ResultMessage>,
    counters: Arc<Counters>,
    subnet: String,
    lookup_type: LookupType
) {
    let mut client = crate::utils::create_client(Some(&subnet), "");
    let mut last_auth_refresh = std::time::Instant::now();
    let auth_refresh_interval = Duration::from_secs(8 * 60 * 60); // Refresh auth every 8 hours
    
    while let Ok(message) = work_rx.recv().await {
        match message {
            WorkerMessage::CheckPhone { record_id, phone, identifier: _identifier, first_name, last_name, pending_counter } => {
                // Get a reference to the counter for decrementing when done
                let decrement_counter = || {
                    if let Some(counter) = &pending_counter {
                        counter.fetch_sub(1, Ordering::SeqCst);
                    }
                };
                
                // Check if we need to refresh authentication
                if last_auth_refresh.elapsed() >= auth_refresh_interval {
                    if let Ok(_) = auth::get_auth_credentials().await {
                        last_auth_refresh = std::time::Instant::now();
                    }
                }
                
                // Skip processing for completion marker
                if phone.starts_with("COMPLETION_MARKER_") {
                    decrement_counter();
                    continue;
                }
                
                // Process the phone number
                counters.requests.fetch_add(1, Ordering::Relaxed);
                
                // Validate phone number
                let parsed_number = match format!("+{}", phone).parse::<phonenumber::PhoneNumber>() {
                    Ok(number) => number,
                    Err(_) => {
                        counters.success.fetch_add(1, Ordering::Relaxed);
                        decrement_counter();
                        continue;
                    }
                };
                
                if !phonenumber::is_valid(&parsed_number) {
                    counters.success.fetch_add(1, Ordering::Relaxed);
                    decrement_counter();
                    continue;
                }
                
                // Similar to the original worker function but streamlined for CSV mode
                for attempt in 0..3 { // Limited retries
                    let lookup_result = match lookup_type {
                        LookupType::Js => js::lookup(&client, &phone, &first_name, &last_name).await,
                        LookupType::NoJS => nojs::lookup(&client, &phone, &first_name, &last_name).await,
                    };
        
                    match lookup_result {
                        Ok(exists) => {
                            counters.success.fetch_add(1, Ordering::Relaxed);
                            
                            if exists {
                                // For phone numbers, verify with fake names to filter false positives
                                match crate::lookup::verify_hit(&client, &phone, &first_name, &last_name).await {
                                    Ok(is_real) => {
                                        if is_real {
                                            // Send hit notification
                                            if let Err(e) = result_tx.send(ResultMessage::Hit {
                                                record_id,
                                                phone: phone.clone(),
                                            }).await {
                                                error!("Failed to send hit: {}", e);
                                            }
                                        }
                                    },
                                    Err(_) => {
                                        // If verification fails, retry
                                        if attempt < 2 {
                                            sleep(Duration::from_millis(100)).await;
                                            continue;
                                        }
                                    }
                                }
                            }
                            
                            // Success or verified non-hit
                            decrement_counter();
                            break;
                        },
                        Err(error) => {
                            let error_str = error.to_string();
                            
                            if error_str == "ratelimited" {
                                counters.ratelimits.fetch_add(1, Ordering::Relaxed);
                                // Get a new client with a different IP
                                client = crate::utils::create_client(Some(&subnet), "");
                                // Add a small delay between retries
                                sleep(Duration::from_millis(100)).await;
                                continue;
                            } else if error_str.contains("botguard") {
                                // Don't try to update botguard token here anymore
                                // Just log an error and increment the error counter
                                error!("Botguard token error: {}", error);
                                counters.errors.fetch_add(1, Ordering::Relaxed);
                                sleep(Duration::from_millis(100)).await;
                                continue;
                            } else {
                                counters.errors.fetch_add(1, Ordering::Relaxed);
                                
                                // If we've tried enough times, move on
                                if attempt >= 2 {
                                    decrement_counter();
                                    break;
                                }
                            }
                        }
                    }
                }
            },
            WorkerMessage::Shutdown => {
                // Exit the worker loop when shutdown is requested
                break;
            }
        }
    }
}