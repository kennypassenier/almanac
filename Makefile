# Variables
BINARY_NAME=cal-stacean
ENV_FILE=.env
ENV_EXAMPLE=.env.example

.PHONY: all build run clean secrets example-env help

# Default target: fetch secrets and then build the project
all: secrets build

# Help command to see available targets
help:
	@echo "Available commands:"
	@echo "  make secrets     - Fetch secrets from Doppler and generate $(ENV_FILE)"
	@echo "  make example-env - Generate $(ENV_EXAMPLE) template from active environment"
	@echo "  make build       - Compile the Rust binary and place it in the project root"
	@echo "  make run         - Run the project using the compiled binary in the root"
	@echo "  make clean       - Remove compiled artifacts, root binary, and env files"

# Fetch secrets from Doppler and write them to a local .env file
secrets:
	@echo "Fetching secrets from Doppler..."
	@doppler secrets download --format=env --no-file > $(ENV_FILE)
	@echo "$(ENV_FILE) successfully generated."

# Generate an .env.example file based on the keys available in Doppler (values stripped)
example-env:
	@echo "Generating $(ENV_EXAMPLE) template..."
	@doppler secrets download --format=env --no-file | sed 's/=.*$$/=your_value_here/' > $(ENV_EXAMPLE)
	@echo "$(ENV_EXAMPLE) successfully generated."

# Build the program in release mode and move the binary to the project root
build: secrets example-env
	@echo "Building Rust binary in release mode..."
	@cargo build --release
	@cp target/release/$(BINARY_NAME) ./$(BINARY_NAME)
	@echo "Build complete. Binary is ready in the project root: ./$(BINARY_NAME)"

# Run the compiled root binary using Doppler to inject secrets into the process environment
run: build
	@echo "Starting daemon from root binary with Doppler environment..."
	@doppler run -- ./$(BINARY_NAME)

# Clean build artifacts and remove local sensitive environment files and root binary
clean:
	@echo "Cleaning up project artifacts..."
	@cargo clean
	@rm -f $(BINARY_NAME)
	@rm -f $(ENV_FILE)
	@rm -f $(ENV_EXAMPLE)
	@echo "Clean complete."