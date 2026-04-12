# Introduction

Welcome to the *Helios FHIR Server Book*, an introductory text about [HL7® FHIR®](https://hl7.org/fhir) and the Helios FHIR Server (HFS). FHIR is the dominant standard for exchanging healthcare data, yet learning it by reading the specification alone is often quite daunting.  The standard spans hundreds of pages across resource definitions, operations, conformance rules, and terminology bindings. This book takes a different approach: you will learn FHIR *by doing*. Each chapter builds on the last, walking you through hands-on exercises with a real FHIR server so that concepts stick through practice rather than passive reading. By the time you finish, you will understand more about FHIR than most practitioners, and you will have a running system to show for it. 

This book does not stop with the basics. It also draws on years of experience building high-performance, complex FHIR implementations for large healthcare institutions. We provide reference architectures for advanced use cases with demanding requirements, so that you can benefit from the knowledge and lessons we have gained in the field.

This is the book we wish we had years ago to help explain and accelerate our customers' work. We hope it does the same for you.

## Who FHIR Is For

FHIR was designed to be accessible to a wide range of participants in healthcare, not just standards experts. The specification itself provides [audience-specific overviews](https://hl7.org/fhir/summary.html) that are worth reading alongside this book. Here is a brief look at each audience and what FHIR offers them.

### Developers

If you build software that touches healthcare data, FHIR gives you a modern, web-native framework to work with. Resources are represented in JSON or XML and exchanged over a RESTful API using standard HTTP methods - `GET` to read, `POST` to create, `PUT` to update, and `DELETE` to remove. Search, history, and batch operations are built in. If you have ever built a REST API, you already understand the interaction model.

FHIR defines roughly 150 resource types - from `Patient` and `Observation` to infrastructure types like `SearchParameter` and `CapabilityStatement`. Every resource shares a [common structure](https://blog.heliossoftware.com/the-r-in-fhir-resources-eda4a2f3612): a type, a logical id, metadata (version, timestamps, tags), an optional human-readable narrative, and the computable data elements themselves. Resources reference each other by URL, forming a web of related clinical and administrative data.

The [Developer Overview](https://hl7.org/fhir/overview-dev.html) in the specification covers the resource model, URL identity, CRUD interactions, search, and the extension/profiling system in detail.

### Clinicians

FHIR models clinical concepts - patients, conditions, medications, observations, care plans - as discrete, reusable building blocks. Think of each resource type as a form template: a `Patient` resource captures demographics, an `Observation` captures a lab result or vital sign, and a `MedicationRequest` captures a prescription. These resources can be exchanged in four ways: through REST APIs (like a filing cabinet you can query), as Documents (frozen snapshots with a cover page), as Messages (event-triggered notifications), or through Services (lightweight, on-demand queries).

Every resource carries a human-readable Narrative alongside its computable data, so clinicians can always review what a system is communicating. The [Clinician Overview](https://hl7.org/fhir/overview-clinical.html) explains how to approach the specification from a clinical perspective.

### Architects

FHIR is not merely a data format - it is a [platform specification](https://blog.heliossoftware.com/fhir-architectural-patterns-ae828b13d40c). It makes very few assumptions about *how* you deploy it, which means it supports a wide range of architectural patterns: from adding a FHIR interface to an existing EHR, to building a FHIR-native clinical data repository, to standing up an analytics pipeline that transforms FHIR resources into tabular data.

The standard is built on six principles: reuse and composability (the 80/20 rule - cover the common case, extend for the rest), scalability (stateless REST), performance (lean resources), usability, data fidelity (strong typing), and implementability. Resources are organized in layers - from foundational infrastructure up through clinical, financial, and specialized domains. The [Architect Overview](https://hl7.org/fhir/overview-arch.html) maps these concepts to enterprise architecture frameworks.

### Patients

FHIR directly benefits patients by giving them a standard way to access their own health data. Through FHIR-enabled patient portals and apps, individuals can aggregate records across providers, track medications and immunizations, verify the accuracy of their data, and share information with caregivers. Regulations like the 21st Century Cures Act and HIPAA in the US, and GDPR in Europe, increasingly mandate this kind of access - and FHIR is the technical standard that makes it practical. The [Patient Overview](https://hl7.org/fhir/overview-patient.html) explains what data is available and how to access it.

## Who This Book Is For

This book assumes you have some programming experience but does not require expertise in any particular language. You do not need to know Rust - the Helios FHIR Server ships as a standalone binary, and most interaction happens through HTTP requests and JSON. Familiarity with REST APIs, the command line, and JSON will help, but the early chapters introduce everything you need.

You do *not* need prior FHIR experience. The book is structured so that each chapter introduces new FHIR concepts right before you use them. If you are already familiar with FHIR and want to skip ahead, the chapter overview below will help you find the right starting point.

## How to Use This Book

This book is written as a sequential tutorial. Later chapters build on concepts and skills from earlier ones, so reading in order will give you the most complete understanding. That said, every chapter is designed to be useful on its own if you need to revisit a specific topic.

The book is organized into four parts:

**Part I - Getting Started** walks through the core concepts, installation, and a hands-on quick start that has a running server with real data in under five minutes.

**Part II - Core Features** covers the main capabilities: FHIRPath expression evaluation, SQL-on-FHIR ViewDefinitions, and the REST API.

**Part III - Advanced Topics** explores the architecture, multi-version FHIR support, Python bindings, code generation, and embedding Helios as a library.

**Part IV - Operations** covers development setup, recipes for common tasks, and contributing to the project.

The **Appendices** provide a CLI reference, a complete FHIRPath function index, a glossary, and the changelog.

There are two kinds of chapters in this book: concept chapters and project chapters. In concept chapters, you will learn about an aspect of FHIR or Helios. In project chapters, you will apply what you have learned by building something together. The project chapters are where the "learning by doing" philosophy comes to life - and they are the reason this book exists. Reading about FHIR is one thing; using it is another entirely.


## Source Code

The source from which this book is generated can be found on [GitHub](https://github.com/HeliosSoftware/hfs/tree/main/book). 

---

*HL7® and FHIR® are registered trademarks of Health Level Seven International.*
