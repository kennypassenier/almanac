# cal-stacean — Universal Google Calendar API Gateway

## Overview

cal-stacean is a universal API gateway for Google Calendar, designed to provide a secure, auditable, and extensible interface for calendar event management. It exposes a RESTful API for CRUD operations on Google Calendar events and integrates with external systems (like Vikunja) via webhooks. The project is production-ready, containerized, and supports secret management via Infisical for secure deployments.

---

## Features
- **Google Calendar CRUD API**: Create, read, update, delete, and search events via REST endpoints.
- **Service Account Authentication**: Uses Google Service Account JWT for secure, automated access.
- **Configurable**: Reads from `config.toml` for calendar defaults, color mapping, and log level.
- **Secrets Management**: Loads sensitive credentials from environment variables, injected securely via Infisical.
- **Vikunja Integration**: Listens for Vikunja task webhooks and syncs them to Google Calendar.
- **Containerized**: Multi-stage Dockerfile for minimal, secure production images.
- **CI/CD**: Automated build, test, tag, and deploy pipeline using GitHub Actions.

---

## Project Structure

- `src/main.rs` — Main application logic, API server, and integrations.
- `config.toml` — Application configuration (calendar defaults, log level, color mapping).
- `Dockerfile` — Multi-stage build for production containers.
- `Makefile` — Local development, build, and release automation.
- `.github/workflows/` — CI/CD pipelines for build, test, Docker, and tagging.

---

## Configuration

### config.toml
- `default_calendar_id`: The calendar to use if none is specified (e.g., "primary" or an email address).
- `default_color_id`: Default color for new events (Google API accepts "1"–"11").
- `log_level`: Minimum log level (trace, debug, info, warn, error).
- `[standard_colors]`: Map of color names to Google Calendar color IDs.

### Environment Variables (.env)
Sensitive values (Google credentials, tokens, etc.) are loaded from a `.env` file, which is generated securely via Infisical in CI/CD or Doppler locally.

---

## Development Workflow

### 1. Prerequisites
- Rust toolchain (stable)
- Docker (for container builds)
- Node.js/NPM (for Infisical CLI)
- Infisical account and service token (for secrets)

### 2. Local Development
1. **Fetch secrets**: `make secrets` (requires Doppler CLI and access)
2. **Build**: `make build` (compiles release binary)
3. **Run**: `make run` (runs with secrets injected)
4. **Clean**: `make clean` (removes build artifacts and env files)
5. **Test**: Use `cargo test` for Rust unit/integration tests

### 3. Docker
- **Build image**: `make docker-build`
- **Login to GHCR**: `make docker-login`
- **Push image**: `make docker-push` (optionally with `AUTO_TAG=1` to bump patch)

### 4. Versioning
- **Bump patch**: `make tag-patch`
- **Bump minor**: `make tag-minor`
- **Push tags**: `git push --tags`

---

## CI/CD Pipeline (GitHub Actions)

### Automated Steps
- On every push to `main`:
  - Fetch secrets from Infisical and generate `.env`
  - Build the Rust binary
  - Upload the binary as a downloadable artifact
  - Build and push Docker images to GHCR
  - Automatically bump the patch version and create a new git tag
- On manual workflow dispatch:
  - Optionally bump and tag versions

### Downloading Build Artifacts
After a workflow run:
1. Go to the "Actions" tab in your GitHub repository.
2. Click on the latest workflow run.
3. Scroll to the "Artifacts" section at the bottom.
4. Click the artifact (e.g., `cal-stacean-binary`) to download the built binary.

---

## Deployment Options

### 1. Docker Compose
- Use the built Docker image from GHCR.
- Mount your own `config.toml` and inject secrets via `.env`.
- Example `docker-compose.yml`:
  ```yaml
  version: '3.8'
  services:
    cal-stacean:
      image: ghcr.io/<your-gh-username>/cal-stacean:latest
      ports:
        - "8080:8080"
      env_file:
        - .env
      volumes:
        - ./config.toml:/etc/cal-stacean/config.toml:ro
  ```

### 2. Kubernetes
- Use the Docker image in a Deployment.
- Mount `config.toml` via ConfigMap and inject secrets via Kubernetes Secrets or Infisical Operator.

### 3. Bare Metal / VM
- Download the binary artifact from GitHub Actions.
- Place `config.toml` and `.env` in the same directory.
- Run: `./cal-stacean`

---

## Infisical Integration

### 1. Create a Service Token
- In Infisical, create a service token with access to your project/environment.

### 2. Add the Token to GitHub
- Go to your repository → Settings → Secrets and variables → Actions → New repository secret.
- Name: `INFISICAL_TOKEN`, Value: (your service token)

### 3. Workflow Usage
- The GitHub Actions workflow uses the Infisical CLI to fetch secrets and generate `.env` before build/deploy steps.
- No secrets are stored in the repo or Docker image.

---

## API Endpoints

- `POST /api/v1/events` — Create event
- `GET /api/v1/events/{id}` — Get event
- `PUT /api/v1/events/{id}` — Update event
- `DELETE /api/v1/events/{id}` — Delete event
- `GET /api/v1/events?query=...` — Search events
- `POST /webhooks/vikunja` — Vikunja webhook integration

---

## Onboarding a New Developer
1. Clone the repository.
2. Install prerequisites (Rust, Docker, Node.js/NPM, Doppler CLI if using locally).
3. Ask for access to Infisical or Doppler secrets.
4. Run `make build` and `make run` to start the API locally.
5. Use the provided endpoints to interact with Google Calendar.
6. For deployment, use Docker or download the binary from GitHub Actions.

---

## Troubleshooting
- **Secrets not loading?** Ensure your `.env` is present and valid, or that Infisical is configured in CI.
- **Docker build fails?** Check for missing dependencies or outdated Rust toolchain.
- **API errors?** Check logs (log level set in `config.toml` or `RUST_LOG`).
- **Need a new release?** Use `make tag-patch` or `make tag-minor`, then push tags.

---

## Contact & Support
For questions, open an issue or contact the repository owner.

---

## License
MIT (or specify your license here)
