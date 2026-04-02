# Foreword

The FHIR standard has come a long way in a relatively short time, from a draft specification championed by a small group of health IT visionaries to becoming the dominant interoperability standard in healthcare. It has brought together EHR vendors, payers, regulators, and research institutions around a common model for health data. What began as an effort to simplify clinical data exchange has grown into something larger — a shared foundation for an entire industry to build on.

At this point, it is easy to see why FHIR gained the traction it did. Who doesn't want a standard that is human-readable, web-native, and flexible enough to model everything from a patient's allergies to a complex clinical trial? The specification you see today combines years of standards work with the practical wisdom of implementers who have lived through the pain of HL7 v2 messages, CDA documents, and proprietary interfaces. FHIR was designed with purpose and crafted with care, offering developers a foundation that makes it easier to build interoperable, reliable healthcare systems.

But the tooling ecosystem around FHIR has historically favored the JVM, and the performance characteristics of many existing implementations reflect choices made for flexibility rather than throughput. As healthcare data workloads have grown more analytical — as organizations have moved from simply exchanging clinical data to building pipelines, running population health queries, and feeding machine learning models — we think there is room for a different set of trade-offs.

The Helios FHIR Server is not our first FHIR server. Our first was a closed-source Java implementation, built over years of work with clients — large health systems, small clinics, research institutions — who needed to move clinical data reliably and at scale. That experience taught us what mattered: correctness under pressure, predictable performance, and tooling that stays out of the way. Java served us well, but as the workloads grew more demanding, we kept running into the same ceilings.

We rebuilt in Rust — and open-sourced it — because we believed that the healthcare community deserved better infrastructure. Rust gave us the control we needed without sacrificing safety: memory safety without a garbage collector, predictable latency, and the kind of compile-time guarantees that matter when you are handling clinical data. And making it open source means that anyone — a hospital system, a research lab, a solo developer building a patient-facing app — can build on that foundation rather than starting from scratch.

This book is the documentation for that effort. It covers the Helios FHIR Server and its companion tools: the FHIRPath expression engine, the SQL-on-FHIR implementation, and the Python bindings. Whether you are building a clinical data pipeline, a standards-compliant healthcare API, or exploring FHIR for the first time, this book offers something for you. By picking it up, you are not just learning a new tool — you are joining a community that believes healthcare data deserves the same quality of infrastructure that the rest of the software industry takes for granted.

Welcome, and we hope you find these tools useful.

— *Steve Munini*
