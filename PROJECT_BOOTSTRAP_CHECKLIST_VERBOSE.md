# Project Bootstrap Checklist: Secure CI/CD with Infisical & GHCR (Verbose)

Use this checklist to set up a new Rust project with secure secrets management (Infisical), automated CI/CD (GitHub Actions), and Docker image publishing to GitHub Container Registry (GHCR).

---

## 1. Repository & Project Initialization
- [ ] **Create a new GitHub repository**
    - Go to https://github.com/new
    - Choose a name, description, and visibility (private/public)
    - Do NOT initialize with a README, .gitignore, or license (we'll add these manually)
- [ ] **Clone the repo locally**
    - Example: `git clone git@github.com:your-username/your-repo.git`
- [ ] **Initialize Rust project**
    - Run: `cargo init --vcs=git` in the repo directory
    - This creates `Cargo.toml`, `src/main.rs`, and initializes a Git repo
- [ ] **Add a .gitignore file for Rust, Docker, and secrets**
    - Create a file named `.gitignore` in the root of your repo
    - Add the following lines (copy exactly):
      ```
      # Rust
      /target/
      **/*.rs.bk
      
      # VSCode
      .vscode/
      
      # Docker
      *.tar
      
      # Secrets
      .env
      .env.*
      !.env.example
      
      # OS
      .DS_Store
      Thumbs.db
      ```
    - This ensures build artifacts, editor settings, and secrets are never committed
- [ ] **Commit initial code**
    - `git add .`
    - `git commit -m "Initial project setup"`
    - `git push`

## 2. Infisical Setup
- [ ] **Create an Infisical account**
    - Go to https://infisical.com or your self-hosted instance
- [ ] **Create a new Infisical project**
    - Name it after your app/repo
- [ ] **Add all required secrets (env vars) to Infisical**
    - Example: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, etc.
    - Use the Infisical UI to add each variable
- [ ] **Create a Service Token**
    - In Infisical, go to Project → Service Tokens → Create
    - Give it access to the correct environment (e.g., prod)
    - Copy the token (starts with `st.`)
- [ ] **(Self-hosted only) Note your INFISICAL_API_URL and projectId**
    - Find your API URL (e.g., `https://infisical.yourdomain.com`)
    - Find your projectId in the Infisical UI (Project Settings)

## 3. GitHub Secrets Configuration
- [ ] **Go to your repo → Settings → Secrets and variables → Actions**
- [ ] **Add secret: `INFISICAL_TOKEN`**
    - Value: your Infisical service token (starts with `st.`)
- [ ] **Add secret: `INFISICAL_PROJECT_ID`**
    - Value: your Infisical projectId (UUID)
- [ ] **(Self-hosted only) Add secret: `INFISICAL_API_URL`**
    - Value: your Infisical API URL (e.g., `https://infisical.yourdomain.com`)
- [ ] **Add secret: `CR_PAT`**
    - Value: a GitHub Personal Access Token with `write:packages` and `repo` scope
    - Create at https://github.com/settings/tokens (classic token, not fine-grained)
    - Store this securely; never commit it

## 4. Docker & GHCR Preparation
- [ ] **Write a multi-stage Dockerfile for your Rust app**
    - Example:
      ```Dockerfile
      FROM rust:1.77 as builder
      WORKDIR /app
      COPY . .
      RUN cargo build --release
      
      FROM debian:bookworm-slim
      WORKDIR /app
      COPY --from=builder /app/target/release/your-binary ./your-binary
      COPY config.toml ./config.toml
      COPY .env.example ./
      EXPOSE 8080
      CMD ["./your-binary"]
      ```
    - Replace `your-binary` with your actual binary name
- [ ] **Test local Docker build**
    - Run: `docker build -t your-image-name .`
    - Ensure it builds without errors
- [ ] **(Optional) Test running container locally**
    - Run: `docker run --env-file .env -p 8080:8080 your-image-name`
    - Confirm the app starts and is accessible

## 5. GitHub Actions Workflow
- [ ] **Create `.github/workflows/github-actions.yml`**
    - Use the working example from this repo or copy the template below
- [ ] **Ensure workflow does the following:**
    - Installs Rust (actions-rs/toolchain)
    - Installs Infisical CLI (via apt, not npm)
    - Fetches secrets from Infisical to `.env` using CLI with `--projectId` and (if self-hosted) `INFISICAL_API_URL`
    - Generates `.env.example` using Infisical CLI
    - Builds Rust binary in release mode
    - Uploads binary as artifact (actions/upload-artifact)
    - Builds and pushes Docker image to GHCR using `CR_PAT` for authentication
    - Uses `paths-ignore` to skip CI for markdown-only changes (see example below)
    - Workflow name is always "CI/CD Pipeline"
- [ ] **Commit and push workflow file**
    - `git add .github/workflows/github-actions.yml`
    - `git commit -m "Add CI/CD workflow"`
    - `git push`

    **Example workflow trigger section:**
    ```yaml
    name: CI/CD Pipeline
    on:
      push:
        branches: [main]
        paths-ignore:
          - 'README.md'
          - '**/*.md'
      workflow_dispatch:
    ```

## 6. README & Documentation
- [ ] **Update `README.md` to include:**
    - How to use Infisical for secrets
    - CI/CD pipeline steps and what each does
    - How to pull and run the Docker image from GHCR
    - Step-by-step onboarding for new developers (prereqs, secrets, build, run)
    - Example `.env.example` and config.toml usage

## 7. First Run & Validation
- [ ] **Push a commit to `main` to trigger the workflow**
- [ ] **Check Actions tab for workflow run and confirm all steps pass**
- [ ] **Check GHCR for published Docker image**
    - Go to https://github.com/your-username/your-repo/pkgs/container/your-repo
- [ ] **Download and test build artifact (binary) from workflow run**
- [ ] **Confirm `.env.example` is up to date and safe to share**

---

## Notes
- For self-hosted Infisical, always set `INFISICAL_API_URL` and use `--projectId` in CLI commands.
- Use a GitHub PAT (not GITHUB_TOKEN) for GHCR pushes in org repos.
- Keep workflow minimal and always named "CI/CD Pipeline" for consistency.
- Never commit real secrets or .env files.
- Always test locally before pushing to main.

---

Copy this file to each new project and check off each item as you go!
