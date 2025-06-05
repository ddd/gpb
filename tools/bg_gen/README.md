# bg_gen

## Installation (linux)

```bash
$ wget https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb
$ sudo apt install xvfb golang-go ./google-chrome-stable_current_amd64.deb -y # Install dependencies
$ go run main.go
```

## API Documentation

### Endpoints

#### 1. Botguard Token Generation Endpoint

- **Endpoint**: `/api/generate_bgtoken`
- **Method**: GET
- **Description**: Generates a botguard token by automating the Google account recovery flow

#### Query Parameters

- **firstName** (optional): Custom first name
- **lastName** (optional): Custom last name

If firstName and lastName are not provided, random values will be generated.

#### Response

- **Success Response**:
  - **Code**: 200 OK
  - **Content**:
    ```json
    {
      "bgToken": "<generated_botguard_token>"
    }
    ```

- **Error Response**:
  - **Code**: 500 Internal Server Error
  - **Content**:
    ```json
    {
      "error": "<error_message>"
    }
    ```

#### Example Usage

```bash
# Using random names
curl http://localhost:7912/api/generate_bgtoken

# Using custom names
curl "http://localhost:7912/api/generate_bgtoken?firstName=John&lastName=Doe"
```

#### 2. Ping Endpoint

- **Endpoint**: `/api/ping`
- **Method**: GET
- **Description**: Simple health check endpoint that returns "pong"

#### Example Usage

```bash
curl http://localhost:7912/api/ping
```