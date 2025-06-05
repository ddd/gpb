use std::process::Command;
use anyhow::{Result, Error, anyhow};

/// Check if the system's ulimit for open files is at least 1 million
pub fn check_ulimit() -> Result<(), Error> {
    // This will only work on Unix-like systems
    #[cfg(unix)]
    {
        // Try to get the current ulimit -n value
        let output = Command::new("sh")
            .arg("-c")
            .arg("ulimit -n")
            .output();
        
        match output {
            Ok(output) => {
                if output.status.success() {
                    // Convert stdout to string and parse as number
                    let ulimit = String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .parse::<u64>();
                    
                    match ulimit {
                        Ok(limit) => {
                            // Check if it's at least 100k
                            if limit < 100_000 {
                                return Err(anyhow!(
                                    "The system's file descriptor limit (ulimit -n) is set to {}, which is too low. \
                                    It needs to be at least 100,000 for this program to work correctly. \
                                    Please run 'ulimit -n 1000000' before starting the program.",
                                    limit
                                ));
                            }
                            // If it's high enough, return OK
                            return Ok(());
                        },
                        Err(_) => {
                            return Err(anyhow!("Failed to parse ulimit output"));
                        }
                    }
                }
            },
            Err(_) => {
                // Command failed, log a warning but continue
                eprintln!("Warning: Could not check ulimit. Make sure 'ulimit -n' is at least 1,000,000.");
            }
        }
    }
    
    // For non-Unix systems or if the command failed, return OK but print a warning
    #[cfg(not(unix))]
    {
        eprintln!("Warning: ulimit check is only available on Unix-like systems. \
                  Make sure your system can handle at least 1,000,000 open files simultaneously.");
    }
    
    Ok(())
}