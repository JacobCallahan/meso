SHELL := /usr/bin/env bash

.DEFAULT_GOAL := help

BIN_DIR ?= $(HOME)/.local/bin
SYSTEMD_USER_DIR ?= $(HOME)/.config/systemd/user
SERVICE_NAME := meso-updraft.service

.PHONY: help build-dev install-dev build-release install-release service-setup

help:
	@echo "Meso local development workflows"
	@echo ""
	@echo "Targets:"
	@echo "  make build-dev       Build debug binaries"
	@echo "  make install-dev     Build debug binaries and install to ~/.local/bin"
	@echo "  make build-release   Build release binaries"
	@echo "  make install-release Build release binaries and install to ~/.local/bin"
	@echo "  make service-setup   Install/enable the meso-updraft systemd user service"
	@echo ""
	@echo "Overrides:"
	@echo "  BIN_DIR=$(BIN_DIR)"
	@echo "  SYSTEMD_USER_DIR=$(SYSTEMD_USER_DIR)"

build-dev:
	cargo build

install-dev: build-dev
	mkdir -p "$(BIN_DIR)"
	cp target/debug/meso "$(BIN_DIR)/"
	@if systemctl --user --quiet is-active "$(SERVICE_NAME)"; then \
		echo "Stopping $(SERVICE_NAME) before replacing meso-updraft..."; \
		systemctl --user stop "$(SERVICE_NAME)"; \
		was_active=1; \
	else \
		was_active=0; \
	fi; \
	cp target/debug/meso-updraft "$(BIN_DIR)/"; \
	if [[ "$$was_active" == "1" ]]; then \
		echo "Starting $(SERVICE_NAME) after replacement..."; \
		systemctl --user start "$(SERVICE_NAME)"; \
	fi

build-release:
	cargo build --release

install-release: build-release
	mkdir -p "$(BIN_DIR)"
	cp target/release/meso "$(BIN_DIR)/"
	@if systemctl --user --quiet is-active "$(SERVICE_NAME)"; then \
		echo "Stopping $(SERVICE_NAME) before replacing meso-updraft..."; \
		systemctl --user stop "$(SERVICE_NAME)"; \
		was_active=1; \
	else \
		was_active=0; \
	fi; \
	cp target/release/meso-updraft "$(BIN_DIR)/"; \
	if [[ "$$was_active" == "1" ]]; then \
		echo "Starting $(SERVICE_NAME) after replacement..."; \
		systemctl --user start "$(SERVICE_NAME)"; \
	fi

service-setup: build-release
	mkdir -p "$(BIN_DIR)" "$(SYSTEMD_USER_DIR)"
	cp target/release/meso-updraft "$(BIN_DIR)/"
	cp data/meso-updraft.service "$(SYSTEMD_USER_DIR)/"
	systemctl --user daemon-reload
	systemctl --user enable --now "$(SERVICE_NAME)"
