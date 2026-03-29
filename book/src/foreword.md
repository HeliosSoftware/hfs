# Foreword

Healthcare data is abundant, complex, and consequential — and working with it should not require heroic engineering effort.

When we set out to build the Helios FHIR Server, we had a simple conviction: a FHIR implementation built in Rust, designed from the start for analytics workloads, could be both fast enough for production clinical systems and expressive enough to serve as a platform for research and data engineering. We wanted something modular — where a developer could reach for just the FHIRPath evaluator, just the SQL-on-FHIR engine, or the full REST server, without pulling in unnecessary dependencies or fighting framework constraints.

The FHIR standard itself is a remarkable achievement. It has brought together EHR vendors, payers, regulators, and research institutions around a common model for health data. But the tooling ecosystem around FHIR has historically favored the JVM, and the performance characteristics of many existing implementations reflect choices made for flexibility rather than throughput. We think there is room for a different set of trade-offs.

This book is the documentation for that effort. It covers the Helios FHIR Server and its companion tools: the FHIRPath expression engine, the SQL-on-FHIR implementation, Python bindings, and the embedding API. Whether you are building a clinical data pipeline, a real-time analytics platform, or a standards-compliant healthcare API, we hope you find these tools useful.

— *Steve Munini*
