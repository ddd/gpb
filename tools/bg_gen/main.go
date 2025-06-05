package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"log"
	"math/rand"
	"net/http"
	"regexp"
	"strings"
	"sync"
	"time"

	cu "github.com/Davincible/chromedp-undetected"
	"github.com/chromedp/cdproto/network"
	"github.com/chromedp/chromedp"
)

// Function to generate a random string of specified length
func randomString(length int) string {
	const charset = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
	seededRand := rand.New(rand.NewSource(time.Now().UnixNano()))
	b := make([]byte, length)
	for i := range b {
		b[i] = charset[seededRand.Intn(len(charset))]
	}
	return string(b)
}

// Function to generate a random 7-digit number as string
func randomPhoneDigits() string {
	seededRand := rand.New(rand.NewSource(time.Now().UnixNano()))
	digits := make([]byte, 7)
	for i := range digits {
		digits[i] = byte(seededRand.Intn(10) + '0')
	}
	return string(digits)
}

// TokenResponse represents the JSON response for the API
type TokenResponse struct {
	BgToken string `json:"bgToken"`
	Error   string `json:"error,omitempty"`
}

// generateBgToken generates a bgToken by automating the Google account recovery flow
func generateBgToken(firstName, lastName string) (string, error) {
	// If firstName or lastName is empty, generate random names
	if firstName == "" {
		firstName = randomString(10)
	}
	if lastName == "" {
		lastName = randomString(10)
	}

	// Generate random phone number with +658 prefix
	randomPhone := "+658" + randomPhoneDigits()

	// Create a new context for use with chromedp
	ctx, cancel, err := cu.New(cu.NewConfig(
		// Run in headless mode for production
		cu.WithHeadless(),
		// Set timeout to 30 seconds
		cu.WithTimeout(30*time.Second),
	))
	if err != nil {
		return "", fmt.Errorf("failed to create chromedp context: %v", err)
	}
	defer cancel()

	// Variable to store the bgToken
	var bgToken string
	var bgTokenMutex sync.Mutex

	// Enable network events
	if err := chromedp.Run(ctx, network.Enable()); err != nil {
		return "", fmt.Errorf("failed to enable network events: %v", err)
	}

	// Create a channel to signal when bgToken is found
	tokenFoundChan := make(chan struct{}, 1)

	// Set up network event listeners
	chromedp.ListenTarget(ctx, func(ev interface{}) {
		switch e := ev.(type) {
		case *network.EventRequestWillBeSent:
			if e.Request != nil && strings.Contains(e.Request.URL, "accounts.google.com/_/lookup/accountlookup") {
				// Print request body if it's a POST request
				if len(e.Request.PostDataEntries) > 0 {
					// Get the bytes from the first PostDataEntry
					postData := e.Request.PostDataEntries[0].Bytes

					// Decode base64 data
					decodedData, err := base64.StdEncoding.DecodeString(string(postData))
					if err != nil {
						// If standard base64 decoding fails, try URL safe variant
						decodedData, err = base64.URLEncoding.DecodeString(string(postData))
						if err != nil {
							log.Printf("Failed to decode base64 data: %v", err)
							return
						}
					}

					// Apply regex to find bgToken
					re := regexp.MustCompile(`&bgRequest=%5B%22username-recovery%22%2C%22([^&]*)%22%5D&azt`)
					matches := re.FindStringSubmatch(string(decodedData))

					if len(matches) > 1 {
						bgTokenMutex.Lock()
						bgToken = strings.Replace(matches[1], "%3C", "<", 1)
						log.Printf("Extracted bgToken: %s\n", bgToken)
						bgTokenMutex.Unlock()

						// Signal that bgToken has been found
						select {
						case tokenFoundChan <- struct{}{}:
						default:
							// Channel already has signal, do nothing
						}
					} else {
						log.Println("No bgToken match found in the data")
					}
				}
			}
		}
	})

	// Execute the account recovery automation flow
	err = chromedp.Run(ctx,
		// Navigate to Google account recovery
		chromedp.Navigate("https://accounts.google.com/signin/v2/usernamerecovery?ddm=1&flowName=GlifWebSignIn&flowEntry=ServiceLogin&hl=en"),
		chromedp.WaitVisible(`/html/body/div[1]/div[1]/div[2]/div/div/div[2]/div/div/div/form/span/section/div/div/div/div/div[1]/div/div[1]/input`),

		// Enter phone number
		chromedp.SendKeys(`/html/body/div[1]/div[1]/div[2]/div/div/div[2]/div/div/div/form/span/section/div/div/div/div/div[1]/div/div[1]/input`, randomPhone),

		// Click next button
		chromedp.WaitVisible(`/html/body/div[1]/div[1]/div[2]/div/div/div[3]/div/div/div/div/button/span`),
		chromedp.Click(`/html/body/div[1]/div[1]/div[2]/div/div/div[3]/div/div/div/div/button/span`),

		// Enter first name
		chromedp.WaitVisible(`/html/body/div[1]/div[1]/div[2]/div/div/div[2]/div/div/div/form/span/section/div/div/div/div[1]/div[1]/div/div[1]/div/div[1]/input`),
		chromedp.SendKeys(`/html/body/div[1]/div[1]/div[2]/div/div/div[2]/div/div/div/form/span/section/div/div/div/div[1]/div[1]/div/div[1]/div/div[1]/input`, firstName),

		// Enter last name
		chromedp.WaitVisible(`/html/body/div[1]/div[1]/div[2]/div/div/div[2]/div/div/div/form/span/section/div/div/div/div[1]/div[2]/div/div[1]/div/div[1]/input`),
		chromedp.SendKeys(`/html/body/div[1]/div[1]/div[2]/div/div/div[2]/div/div/div/form/span/section/div/div/div/div[1]/div[2]/div/div[1]/div/div[1]/input`, lastName),

		// Click final button to submit form
		chromedp.WaitVisible(`/html/body/div[1]/div[1]/div[2]/div/div/div[3]/div/div/div/div/button/span`),
		chromedp.Click(`/html/body/div[1]/div[1]/div[2]/div/div/div[3]/div/div/div/div/button/span`),

		// Wait for the completion page
		chromedp.WaitVisible(`/html/body/div[1]/div[1]/div[2]/div/div/div[1]/div[2]/h1/span`),
	)

	if err != nil {
		return "", fmt.Errorf("automation error: %v", err)
	}

	// Wait for either bgToken to be found or timeout
	select {
	case <-tokenFoundChan:
		// bgToken has been found, return it
		bgTokenMutex.Lock()
		token := bgToken
		bgTokenMutex.Unlock()
		return token, nil
	case <-time.After(10 * time.Second):
		return "", fmt.Errorf("timeout waiting for bgToken")
	}
}

// handleGenerateBgToken handles the /api/generate_bgtoken endpoint
func handleGenerateBgToken(w http.ResponseWriter, r *http.Request) {
	// Set response headers
	w.Header().Set("Content-Type", "application/json")

	// Only allow GET requests
	if r.Method != http.MethodGet {
		w.WriteHeader(http.StatusMethodNotAllowed)
		json.NewEncoder(w).Encode(TokenResponse{
			Error: "Method not allowed",
		})
		return
	}

	// Extract firstName and lastName from query parameters
	firstName := r.URL.Query().Get("firstName")
	lastName := r.URL.Query().Get("lastName")

	// Generate bgToken with provided or random names
	bgToken, err := generateBgToken(firstName, lastName)
	if err != nil {
		w.WriteHeader(http.StatusInternalServerError)
		json.NewEncoder(w).Encode(TokenResponse{
			Error: err.Error(),
		})
		return
	}

	// Return successful response
	w.WriteHeader(http.StatusOK)

	// Use json.Marshal to create the JSON bytes
	responseBytes, err := json.Marshal(TokenResponse{
		BgToken: bgToken,
	})
	if err != nil {
		w.WriteHeader(http.StatusInternalServerError)
		json.NewEncoder(w).Encode(TokenResponse{
			Error: "Failed to marshal response",
		})
		return
	}

	// Replace the escaped characters in the JSON string
	jsonStr := string(responseBytes)
	jsonStr = strings.Replace(jsonStr, "\\u003c", "<", -1)

	// Write the modified JSON response
	w.Write([]byte(jsonStr))
}

// handlePing handles the /api/ping endpoint
func handlePing(w http.ResponseWriter, r *http.Request) {
	// Only allow GET requests
	if r.Method != http.MethodGet {
		w.WriteHeader(http.StatusMethodNotAllowed)
		w.Write([]byte("Method not allowed"))
		return
	}

	// Return "pong"
	w.WriteHeader(http.StatusOK)
	w.Write([]byte("pong"))
}

func main() {
	// Define API routes
	http.HandleFunc("/api/generate_bgtoken", handleGenerateBgToken)
	http.HandleFunc("/api/ping", handlePing)

	// Log server start
	log.Println("Starting server on :7912")
	log.Println("API endpoints:")
	log.Println("- GET http://localhost:7912/api/generate_bgtoken")
	log.Println("- GET http://localhost:7912/api/ping")
	log.Println("Optional parameters for generate_bgtoken: firstName, lastName")

	// Start HTTP server
	if err := http.ListenAndServe(":7912", nil); err != nil {
		log.Fatalf("Failed to start server: %v", err)
	}
}
