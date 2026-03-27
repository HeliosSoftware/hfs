# Helios FHIR Server

The Helios FHIR Server is an implementation of the [HL7® FHIR®](https://hl7.org/fhir) standard, built in Rust for high performance and optimized for clinical analytics workloads. It provides modular components that can be run as standalone command-line tools, integrated as microservices, or embedded directly into your data analytics pipeline.

## Why Helios?

- **Blazing Fast**: Built in Rust for maximum performance and minimal resource usage
- **Analytics-First**: Optimized for clinical data analytics and research workloads
- **Modular Design**: Use only what you need — from FHIRPath expressions to a full FHIR server
- **Multi-Version Support**: Work with R4, R4B, R5, and R6 data in the same application
- **Developer Friendly**: Excellent error messages, comprehensive tooling, and CLI tools

## What People Build with Helios

- **Clinical Research Platforms**: Transform FHIR data into research-ready datasets using SQL-on-FHIR
- **Real-time Analytics Dashboards**: Process streaming FHIR data for operational insights
- **Data Quality Tools**: Validate and profile FHIR data using FHIRPath expressions
- **ETL Pipelines**: Extract and transform FHIR data for data warehouses and lakes
- **Healthcare APIs**: Build high-performance FHIR-compliant REST APIs
- **Healthcare Analytics**: Analyze patient cohorts at scale

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
