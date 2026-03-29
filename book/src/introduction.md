# Introduction

The Helios FHIR Server is an implementation of the [HL7® FHIR®](https://hl7.org/fhir) standard, built in Rust for high performance and optimized for clinical analytics workloads. It provides modular components that can be run as standalone command-line tools, integrated as microservices, or embedded directly into your data analytics pipeline.

## Who This Book Is For

This book is for developers and data engineers who want to work with FHIR data using Helios. You do not need prior FHIR experience — the early chapters introduce the standard from first principles. Familiarity with at least one systems or scripting language will help, but no Rust knowledge is required to run and configure the server.

If you are already familiar with FHIR and want to get started quickly, jump to [Installation](ch02-installation.md) and then the [Quick Start](ch03-quickstart.md).

## What This Book Covers

**Part I — Getting Started** walks through the core concepts, installation, and a hands-on quick start that has a running server with real data in under five minutes.

**Part II — Core Features** covers the main capabilities: FHIRPath expression evaluation, SQL-on-FHIR ViewDefinitions, and the REST API.

**Part III — Advanced Topics** explores the architecture, multi-version FHIR support, Python bindings, code generation, and embedding Helios as a library.

**Part IV — Operations** covers development setup, recipes for common tasks, and contributing to the project.

The **Appendices** provide a CLI reference, a complete FHIRPath function index, a glossary, and the changelog.

## Components

The project ships several standalone tools:

| Component | Description |
|-----------|-------------|
| [`hfs`](components/hfs-server.md) | Main FHIR REST server |
| [`fhirpath-cli` / `fhirpath-server`](components/fhirpath.md) | FHIRPath expression evaluation |
| [`sof-cli` / `sof-server`](components/sql-on-fhir.md) | SQL-on-FHIR ViewDefinition transformation |
| [`pysof`](components/pysof.md) | Python bindings for SQL-on-FHIR |
| [`helios-cds-hooks`](components/cds-hooks.md) | CDS Hooks protocol types |

## FHIR Version Support

| Version | Status |
|---------|--------|
| FHIR R4 (4.0.1) | ✅ Default |
| FHIR R4B (4.3.0) | ✅ Supported |
| FHIR R5 (5.0.0) | ✅ Supported |
| FHIR R6 (6.0.0-ballot2) | ✅ Supported |

---

*HL7® and FHIR® are registered trademarks of Health Level Seven International.*
