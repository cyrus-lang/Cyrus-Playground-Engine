# Cyrus Playground API

REST API for executing Cyrus code remotely. Can be used in web applications, mobile apps, or any HTTP client.

## Endpoints

### POST `/api/execute`

Execute Cyrus code and get the result.

**Request:**
```json
{
  "code": "import std::libc{printf};\n\npub fn main() {\n    printf(\"Hello!\\n\");\n}"
}
```

**Response (Success):**
```json
{
  "success": true,
  "stdout": "Hello!\n",
  "stderr": "",
  "execution_time": 0.23
}
```

**Response (Compilation Error):**
```json
{
  "success": false,
  "stdout": "",
  "stderr": "Error: ...",
  "execution_time": 0.15
}
```

**Response (API Error):**
```json
{
  "error": "Code cannot be empty"
}
```

**Status Codes:**
- `200 OK` - Code executed (check `success` field for compilation result)
- `400 Bad Request` - Invalid request (empty code, too long)
- `500 Internal Server Error` - Binary not ready or execution failed

### GET `/api/health`

Health check endpoint.

**Response:**
```
OK
```

## Limits

- Max code size: 50 KB
- Max request body: 100 KB
- CORS: Enabled for all origins
- Rate limiting: Not implemented (add reverse proxy for production)

## Usage Examples

### cURL
```bash
curl -X POST http://localhost:3000/api/execute \
  -H "Content-Type: application/json" \
  -d '{
    "code": "import std::libc{printf};\n\npub fn main() {\n    printf(\"Hello from Cyrus!\\n\");\n}"
  }'
```

### JavaScript (fetch)
```javascript
const response = await fetch('http://localhost:3000/api/execute', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
  },
  body: JSON.stringify({
    code: `import std::libc{printf};

pub fn main() {
    printf("Hello from Cyrus!\\n");
}`
  })
});

const result = await response.json();
console.log(result);
```

### Python
```python
import requests

response = requests.post('http://localhost:3000/api/execute', json={
    'code': '''import std::libc{printf};

pub fn main() {
    printf("Hello from Cyrus!\\n");
}'''
})

print(response.json())
```

## Running the API

```bash
# Set environment variables
export PORT=3000
export RUST_LOG=info

# Run the API server
cargo run --bin cyrus-api
```

Or:
```bash
source .env
cargo run --bin cyrus-api
```

## Production Deployment

For production, consider:

1. **Reverse Proxy**: Use Nginx or Caddy
2. **Rate Limiting**: Implement per-IP limits
3. **Sandbox**: Add container-based isolation (Docker, firejail, etc.)
4. **Monitoring**: Add metrics and logging
5. **HTTPS**: Use TLS certificate
6. **Authentication**: Add API keys if needed

Example Nginx config:
```nginx
server {
    listen 80;
    server_name api.cyrus-lang.com;

    location /api/ {
        proxy_pass http://localhost:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        
        # Rate limiting
        limit_req zone=api burst=10 nodelay;
    }
}
```

## Docker Support

Coming soon...

## Security Notes

- Code executes in temporary files
- No persistent storage
- Consider adding execution timeout
- Add sandbox for production use
- Binary is auto-updated daily
