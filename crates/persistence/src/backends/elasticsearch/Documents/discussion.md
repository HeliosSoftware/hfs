Introduction
As I write this in early 2026, I don't think it is an understatement to say that the opportunities and impact that are upon us with AI in healthcare feels like a Cambrian Explosion moment. Healthcare professionals, administrators, and patients alike will be increasingly chatting with, talking directly to, and collaborating with artificial intelligence software systems in entirely new ways. This will need to be done safely and carefully.
What worked five years ago, or even two years ago, is increasingly inadequate for the demands of clinical AI, population health analytics, and real-time decision support. For technical architects navigating this shift, the challenge isn't just scaling storage; it's rethinking the entire data architecture.
This discussion document shares my thoughts about an approach to persistence for the Helios FHIR Server.
This document is an architecture strategy document. In other words, it describes the main motivating direction, building blocks, and key technology ingredients that will makeup the persistence design for the Helios FHIR Server. It is not intended to be a comprehensive set of requirements and design, but instead contains enough of a starting point such that readers can understand our approach to persistence, and understand why we decided to make the decisions that we did.
Who should read this?
The Helios FHIR Server is open source software, and is being developed in the open. If you have some interest in persistence design for healthcare software - this document is for you!
My hope is that you will think about the contents of this document, comment and provide feedback!
AI Is Driving New Requirements on Data
AI workloads have upended traditional assumptions about data access patterns. Training models demand sustained high-throughput reads across massive datasets, while inference requires low-latency access to distributed data sources. In healthcare, this is compounded by the explosive growth of unstructured data. Radiology images, pathology slides, genomic sequences, clinical notes, and waveform data from monitoring devices to name a few. Structured EHR data, once the center of gravity, is increasingly extracted from the EMR and compared with other external data sources. Architectures optimized for transactional workloads simply cannot deliver the performance AI pipelines require, and retrofitting them is often a losing battle.
Separation of Storage and Compute
Decoupling storage from compute has moved from a cloud-native best practice to an architectural necessity, yet many FHIR server implementations haven't caught up. While cloud-based analytics platforms routinely embrace this separation, transactional FHIR servers often remain tightly coupled to their persistence layers, treating database and application as an inseparable unit. This creates painful trade-offs: over-provisioning compute to get adequate storage, or vice versa. A modern FHIR server must separate these concerns as a core architectural principle, allowing the API layer to scale horizontally for request throughput while the persistence layer scales independently for capacity and query performance. In healthcare AI workloads, this separation is especially critical. Spin up GPU clusters for model training without provisioning redundant storage, or expand storage for imaging archives without paying for idle compute. The persistence layer becomes a service with its own scaling characteristics rather than a monolithic dependency. This separation is now expected as a defining characteristic of production-ready FHIR infrastructure.
Medallion Architecture Within FHIR Persistence
We have seen our largest petabyte-scale customers transition to a Medallion Architecture strategy for their FHIR data. The bronze layer represents resources as received, preserving original payloads, source system identifiers, and ingestion metadata for auditability and replay. The silver layer applies normalization: terminology mapping, reference resolution, deduplication of resources that represent the same clinical entity, and enforcement of business rules that go beyond FHIR validation. The gold layer materializes optimized views for specific consumers, denormalized patient summaries for clinical applications, flattened tabular projections for analytics, or pre-computed feature sets for ML pipelines.
Hybrid and Multi-Cloud Architectures
The reality for most health IT systems is a hybrid footprint: on-premises data centers housing legacy systems and sensitive workloads, cloud platforms providing elastic compute for AI and analytics, and edge infrastructure at clinical sites. Multi-cloud strategies add another dimension, whether driven by M&A activity, best-of-breed vendor selection, or risk diversification.
Security-First and Zero-Trust Patterns in FHIR Persistence
The persistence layer is where FHIR data lives at rest, making it the most critical surface for security enforcement. Zero-trust principles must be embedded in the persistence design itself, not just the API layer above it. This means encryption at rest as a baseline, but also fine-grained access control at the resource, compartment or even finer-grained levels - ensuring that database-level access cannot bypass FHIR authorization semantics. Audit logging must capture all persistence operations with sufficient detail for HIPAA accounting-of-disclosures requirements. This typically means persisting AuditEvent resources to a separately controlled store. Consent enforcement, particularly for sensitive resource types like mental health or substance abuse records under 42 CFR Part 2, often requires persistence-layer support through segmentation, tagging, or dynamic filtering. Treating security as an API-layer concern while leaving the persistence layer permissive creates unacceptable risk.
Data Retention, Tiering, and Cost Optimization
FHIR persistence layers accumulate data over years and decades. Version history, provenance records, and audit logs all create significant cost pressure. Intelligent tiering within the persistence layer moves older resource versions and infrequently accessed resources to lower-cost storage classes while keeping current data on performant storage. The architectural challenge is maintaining query semantics across tiers: a search that spans active and archived resources should work transparently, even if archived retrieval is slower. Retention policies must account for regulatory requirements that vary by resource type. Imaging studies may have different retention mandates than clinical notes. A well-designed persistence layer makes tiering a configuration concern rather than an architectural constraint.
Different Data Technologies for Different Problems
A FHIR persistence layer that commits to a single storage technology is making a bet that one tool can serve all masters. This is a bet that rarely pays off as requirements evolve. The reality is that different access patterns, query types, and workloads have fundamentally different performance characteristics, and no single database technology optimizes for all of them. A patient lookup by identifier, a population-level cohort query, a graph traversal of care team relationships, and a semantic similarity search for clinical trial matching across different terminology code systems are all legitimate operations against FHIR data, yet each performs best on a different underlying technology.
Modern FHIR persistence architectures increasingly embrace polyglot persistence, which means routing data to the storage technology best suited for how that data will be accessed, while maintaining a unified FHIR API layer above.
* Relational Databases remain the workhorse for transactional FHIR operations, offering ACID guarantees, mature tooling, and well-understood query optimization for structured data with predictable access patterns.
* NoSQL Databases - particularly document stores - align naturally with FHIR's resource model, persisting resources as complete documents without the impedance mismatch of relational decomposition, and scaling horizontally for high-throughput ingestion. Additionally, Cassandra has been exceptional at handling web-scale data requirements without breaking the bank.
* Data Lakes provide cost-effective, schema-flexible storage for raw FHIR resources and bulk exports, serving as the foundation for large-scale analytics and ML training pipelines that need to process millions of resources.
* Data Warehouses deliver optimized analytical query performance over structured, transformed FHIR data, enabling population health analytics, quality measure computation, and business intelligence workloads that would overwhelm transactional systems.
* Graph Databases excel at traversing relationships. Patient to provider to organization to care team is an example relationship pathway that are represented as references in FHIR but are expensive to navigate through recursive joins in relational systems.
* Vector Databases enable semantic search and similarity matching over embedded representations of clinical text, supporting AI use cases like similar-patient retrieval, terminology matching, and contextual search that go beyond keyword-based FHIR queries.
* Block Storage provides the high-performance, low-latency foundation for database engines themselves, while also serving large binary attachments, imaging data, scanned documents, and waveforms that are referenced by FHIR resources but impractical to store within the resource payload.
The architectural discipline is not choosing one technology but designing the abstraction layer that routes FHIR operations to the appropriate backend while maintaining consistency, security, and a coherent developer experience.
Positioning the Helios FHIR Server in the FHIR Server Landscape
￼
The FHIR server landscape can be understood along two architectural dimensions: how tightly the implementation is coupled to its storage technology, and whether the system supports multiple specialized data stores or requires a single backend.
The vertical axis distinguishes between servers with tightly-coupled persistence where the implementation is deeply intertwined with a specific database technology, and those offering an extensible interface layer that abstracts storage concerns behind well-defined interfaces. A FHIR Server built directly on JPA (Java Persistence API) is such an example, meaning its data access patterns, query capabilities, and performance characteristics are fundamentally shaped by relational database assumptions. In contrast, an extensible interface layer defines traits or interfaces that can be implemented for any storage technology, allowing the same FHIR API to sit atop different backends without rewriting core logic.
The horizontal axis captures the difference between single storage backend architectures and polyglot persistence. Polyglot persistence is an architectural pattern where different types of data are routed to the storage technologies best suited for how that data will be accessed. For example, a polyglot system might store clinical documents in an object store optimized for large binary content, maintain patient relationships in a graph database for efficient traversal, and keep structured observations in a columnar store for fast analytical queries all while presenting a unified FHIR API to consuming applications. Most existing FHIR servers force all resources into a single database, sacrificing performance and flexibility for implementation simplicity.
The Helios FHIR Server occupies the upper-right quadrant: it combines a trait-based, open-source interface layer built in Rust with native support for polyglot persistence. This architecture allows organizations to optimize storage decisions for their specific access patterns while maintaining full FHIR compliance at the API layer.
Decomposing the FHIR Specification: Separation of Concerns in Persistence Design
The FHIR specification is vast. It defines resource structures, REST interactions, search semantics, terminology operations, versioning behavior, and much more. A monolithic interface, or trait that attempts to capture all of this becomes unwieldy, difficult to implement, and impossible to optimize for specific storage technologies. The Helios FHIR Server persistence design takes a different approach: decompose the specification into cohesive concerns, express each as a focused trait, and compose them to build complete storage backends.
Learning from Diesel: Type-Safe Database Abstractions
Before diving into our trait design, it's worth examining what we can learn from Diesel, Rust's most mature database abstraction layer. Diesel has solved many of the problems we face - multi-backend support, compile-time query validation, extensibility, and its design choices offer valuable lessons.
Backend Abstraction via Traits, Not Enums: Diesel defines a Backend trait that captures the differences between database systems (PostgreSQL, MySQL, SQLite) without coupling to specific implementations. The Backend trait specifies how SQL is generated, how bind parameters are collected, and how types are mapped. This allows new backends to be added without modifying core code. This is exactly what we need for polyglot FHIR persistence.
QueryFragment for Composable SQL Generation: Diesel's QueryFragment trait represents any piece of SQL that can be rendered. A WHERE clause, a JOIN, an entire SELECT statement all implement QueryFragment. This composability lets complex queries be built from simple pieces. For FHIR search, we can adopt a similar pattern: each search parameter modifier becomes a fragment that can be composed into complete queries.
Type-Level Query Validation: Diesel catches many errors at compile time by encoding schema information in the type system. While we can't achieve the same level of compile-time validation for dynamic FHIR queries, we can use Rust's type system to ensure that storage backends only claim to support operations they actually implement.
MultiConnection for Runtime Backend Selection: Diesel's #[derive(MultiConnection)] generates an enum that wraps multiple connection types, dispatching operations to the appropriate backend at runtime. This pattern directly applies to polyglot persistence. We can route FHIR operations to different backends based on query characteristics.
Extensibility via sql_function! and Custom Types: Diesel makes it trivial to add custom SQL functions and types. For FHIR, this translates to extensibility for custom search parameters, terminology operations, and backend-specific optimizations.
The Core Resource Storage Trait
At the foundation is the ResourceStorage trait, which handles the fundamental persistence of FHIR resources. This trait intentionally knows nothing about search, nothing about REST semantics, nothing about transactions. It simply stores and retrieves resources by type and identifier.
Multitenancy is not optional in this design. Every operation requires a TenantContext, making it impossible at the type level to accidentally execute a query without tenant scoping. There is no "escape hatch" that bypasses tenant isolation.
use async_trait::async_trait;
use serde_json::Value;

/// Represents a stored FHIR resource with metadata.
pub struct StoredResource {
    pub resource_type: String,
    pub id: String,
    pub version_id: String,
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub tenant_id: TenantId,
    pub resource: Value,
}

/// Core trait for resource storage operations.
/// 
/// All operations are tenant-scoped. There is no non-tenant code path - 
/// the type system enforces that tenant context is always provided.
#[async_trait]
pub trait ResourceStorage: Send + Sync {
    /// Creates a new resource within a tenant's scope, assigning an ID if not provided.
    async fn create(
        &self,
        tenant: &TenantContext,
        resource: &Value,
    ) -> Result<StoredResource, StorageError>;

    /// Reads the current version of a resource within a tenant's scope.
    /// Returns NotFound if the resource exists but belongs to a different tenant.
    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> Result<Option<StoredResource>, StorageError>;

    /// Updates a resource within a tenant's scope, returning the new version.
    async fn update(
        &self,
        tenant: &TenantContext,
        resource: &Value,
    ) -> Result<StoredResource, StorageError>;

    /// Deletes a resource within a tenant's scope (soft delete preserving history where supported).
    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> Result<(), StorageError>;

    /// Returns the storage backend identifier for logging and diagnostics.
    fn backend_name(&self) -> &'static str;
}

Notice what's absent: there's no if_match parameter for optimistic concurrency, no version-specific reads, no history. Those capabilities belong to separate traits that extend the base functionality. A storage backend that doesn't support versioning simply doesn't implement the versioning trait.
Multitenancy: A Cross-Cutting Concern
Multitenancy has downstream implications for every layer of a FHIR server, from indexing strategy to reference validation to search semantics. By requiring tenant context at the lowest storage layer, we ensure that isolation guarantees propagate upward through the entire system.
Isolation Strategies
There are three fundamental approaches to tenant isolation, each with different trade-offs:
* Database-per-tenant: Strongest isolation, simplest security model, easier compliance story. The downside is operational overhead that grows linearly with tenants. Connection pool management becomes complex, and schema migrations are painful at scale.
* Schema-per-tenant: Good isolation within a single database instance, allows tenant-specific indexing. PostgreSQL handles this well. Still has schema migration coordination challenges.
* Shared schema with tenant discriminator: Most operationally efficient at scale, single migration path. The risk is that every query must include tenant filtering. One missed WHERE clause and you have a data breach.
For SQL-backed FHIR persistence, the shared schema approach with a tenant_id discriminator is pragmatic, but the enforcement layer must be airtight - you literally cannot construct a storage operation without providing tenant context.
Tenant Context as a Type-Level Guarantee
Borrowing from Diesel's approach to type safety, we can make tenant context explicit in the type system. Rather than passing tenant IDs as strings that might be forgotten, we create a wrapper type that must be present for any storage operation:
/// A validated tenant context. Operations that access tenant data
/// require this type, making it impossible to forget tenant filtering.
#[derive(Debug, Clone)]
pub struct TenantContext {
    tenant_id: TenantId,
    /// Permissions determine what operations are allowed
    permissions: TenantPermissions,
    /// Whether this context can access shared/system resources
    can_access_shared: bool,
}

/// The system tenant for shared resources (terminology, conformance)
pub const SYSTEM_TENANT: TenantId = TenantId::system();

/// Marker trait for operations that are tenant-scoped
pub trait TenantScoped {
    fn tenant(&self) -> &TenantContext;
}

Shared Resources and the System Tenant
CodeSystems, ValueSets, StructureDefinitions, and other conformance resources are typically shared across tenants. We designate a "system" tenant that holds these shared resources:
/// Determines whether a resource type should be tenant-specific or shared.
pub trait ResourceTenancy {
    /// Returns the tenancy model for a resource type.
    fn tenancy_model(&self, resource_type: &str) -> TenancyModel;
}

pub enum TenancyModel {
    /// Resource is always tenant-specific (e.g., Patient, Observation)
    TenantScoped,
    /// Resource is always shared (e.g., CodeSystem, ValueSet)
    Shared,
    /// Resource can be either, determined by business rules
    Configurable,
}

Index Design for Multitenancy
Search performance in a multitenant system depends critically on index design. The tenant_id must be the leading column in composite indexes:
-- Good: tenant_id leads, enabling efficient tenant-scoped queries
CREATE INDEX idx_patient_identifier ON patient (tenant_id, identifier_system, identifier_value);

-- Bad: tenant_id not leading, will scan all tenants
CREATE INDEX idx_patient_identifier ON patient (identifier_system, identifier_value, tenant_id);

Versioning as a Separate Concern
FHIR's versioning model is sophisticated: every update creates a new version, version IDs are opaque strings, and the vread interaction retrieves historical versions. Not all storage backends can efficiently support this. An append-only data lake handles versioning naturally; a key-value store might not.
/// Adds version-aware operations to base storage.
#[async_trait]
pub trait VersionedStorage: ResourceStorage {
    /// Reads a specific version of a resource within a tenant's scope.
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> Result<Option<StoredResource>, StorageError>;

    /// Updates with optimistic concurrency control.
    /// Fails with VersionConflict if current version doesn't match expected.
    async fn update_with_match(
        &self,
        tenant: &TenantContext,
        resource: &Value,
        expected_version: &str,
    ) -> Result<StoredResource, StorageError>;
}

History: Building on Versioning
History access naturally extends versioning. If a backend can read specific versions, it can also enumerate them:
/// Provides access to resource history.
#[async_trait]
pub trait HistoryProvider: VersionedStorage {
    /// Returns the history of a specific resource within a tenant's scope.
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> Result<HistoryBundle, StorageError>;

    /// Returns the history of all resources of a type within a tenant's scope.
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> Result<HistoryBundle, StorageError>;

    /// Returns the history of all resources within a tenant's scope.
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> Result<HistoryBundle, StorageError>;
}

/// Parameters for history queries, matching FHIR's _since, _at, _count parameters.
pub struct HistoryParams {
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub at: Option<chrono::DateTime<chrono::Utc>>,
    pub count: Option<usize>,
}

The trait hierarchy HistoryProvider: VersionedStorage: ResourceStorage means that any storage backend supporting history automatically supports versioned reads and basic CRUD - all within tenant boundaries. The type system enforces this relationship.
The Search Abstraction: Decomposing FHIR's Query Model
Search is where the FHIR specification becomes genuinely complex. There are eight search parameter types (number, date, string, token, reference, quantity, uri, composite), sixteen modifiers (:exact, :contains, :not, :missing, :above, :below, :in, :not-in, :of-type, :identifier, :text, :code-text, :text-advanced, :iterate, plus resource type modifiers on references), six comparison prefixes (eq, ne, lt, le, gt, ge, sa, eb, ap), chained parameters, reverse chaining (_has), _include and _revinclude directives, and advanced filtering via _filter. A single search query can combine all of these all while respecting tenant boundaries.
Modeling search as a single trait would be a mistake. Instead, we decompose it into layers - and here, Diesel's QueryFragment pattern proves invaluable.
The SearchFragment Pattern (Inspired by Diesel's QueryFragment)
Diesel's QueryFragment trait allows any piece of SQL to be composable. We adapt this pattern for FHIR search, creating fragments that can be combined into complete search queries:
/// A fragment of a FHIR search that can be rendered to a backend-specific query.
/// Inspired by Diesel's QueryFragment pattern.
pub trait SearchFragment<B: SearchBackend> {
    /// Renders this fragment to the backend's query representation.
    fn apply(&self, builder: &mut B::QueryBuilder) -> Result<(), SearchError>;
    
    /// Whether this fragment can be efficiently evaluated by the backend.
    /// Returns false if the backend would need to do post-filtering.
    fn is_native(&self, backend: &B) -> bool;
    
    /// Estimated cost of evaluating this fragment (for query planning).
    fn estimated_cost(&self, backend: &B) -> QueryCost;
}

/// A search backend that can evaluate SearchFragments.
pub trait SearchBackend: Send + Sync {
    type QueryBuilder;
    type QueryResult;
    
    /// Creates a new query builder for this backend.
    fn query_builder(&self, resource_type: &str) -> Self::QueryBuilder;
    
    /// Executes a built query.
    async fn execute(&self, query: Self::QueryBuilder) -> Result<Self::QueryResult, SearchError>;
}

Each search modifier becomes a fragment that knows how to render itself:
/// Fragment for the :exact modifier on string parameters.
pub struct ExactStringMatch {
    pub parameter: String,
    pub path: FhirPath,
    pub value: String,
}

impl<B: SearchBackend> SearchFragment<B> for ExactStringMatch 
where
    B: SupportsExactMatch,
{
    fn apply(&self, builder: &mut B::QueryBuilder) -> Result<(), SearchError> {
        builder.add_exact_string_match(&self.path, &self.value)
    }
    
    fn is_native(&self, _backend: &B) -> bool {
        true  // Most backends support exact string matching natively
    }
    
    fn estimated_cost(&self, backend: &B) -> QueryCost {
        backend.cost_for_exact_match(&self.path)
    }
}

/// Fragment for the :above modifier on token parameters (terminology subsumption).
pub struct SubsumesMatch {
    pub parameter: String,
    pub path: FhirPath,
    pub system: String,
    pub code: String,
}

impl<B: SearchBackend> SearchFragment<B> for SubsumesMatch 
where
    B: SupportsTerminologySearch,
{
    fn apply(&self, builder: &mut B::QueryBuilder) -> Result<(), SearchError> {
        builder.add_subsumes_match(&self.path, &self.system, &self.code)
    }
    
    fn is_native(&self, backend: &B) -> bool {
        // Only native if the backend has integrated terminology support
        backend.has_native_terminology()
    }
    
    fn estimated_cost(&self, backend: &B) -> QueryCost {
        if self.is_native(backend) {
            backend.cost_for_subsumption(&self.path)
        } else {
            QueryCost::RequiresExpansion  // Will need to expand the code set first
        }
    }
}

Search Parameter Types
First, we model the search parameter types and their associated matching logic:
/// The type of a search parameter, determining matching semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchParamType {
    Number,
    Date,
    String,
    Token,
    Reference,
    Quantity,
    Uri,
    Composite,
    Special,
}

/// Comparison prefixes for ordered types (number, date, quantity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchPrefix {
    #[default]
    Eq,  // equals (default)
    Ne,  // not equals
    Lt,  // less than
    Le,  // less than or equals
    Gt,  // greater than
    Ge,  // greater than or equals
    Sa,  // starts after
    Eb,  // ends before
    Ap,  // approximately
}

/// Modifiers that alter search behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchModifier {
    Exact,           // String: case-sensitive, full match
    Contains,        // String: substring match
    Text,            // Token/Reference: search display text
    TextAdvanced,    // Token/Reference: advanced text search
    CodeText,        // Token: search code text
    Not,             // Token: negation
    Missing(bool),   // All: test for presence/absence
    Above,           // Token/Reference/Uri: hierarchical above
    Below,           // Token/Reference/Uri: hierarchical below
    In,              // Token: value set membership
    NotIn,           // Token: value set non-membership
    OfType,          // Token (Identifier): type-qualified search
    Identifier,      // Reference: search by identifier
    Type(String),    // Reference: restrict to resource type
    Iterate,         // _include/_revinclude: recursive inclusion
}

The Core Search Trait
The base search trait handles fundamental query execution without advanced features:
/// A parsed search parameter with its value and modifiers.
#[derive(Debug, Clone)]
pub struct SearchParameter {
    pub name: String,
    pub param_type: SearchParamType,
    pub modifier: Option<SearchModifier>,
    pub prefix: Option<SearchPrefix>,
    pub values: Vec<String>,  // Multiple values = OR
}

/// A complete search query with all parameters.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Filter parameters (AND-joined)
    pub parameters: Vec<SearchParameter>,
    /// Sort specifications
    pub sort: Vec<SortSpec>,
    /// Pagination
    pub count: Option<u32>,
    pub offset: Option<u32>,
    /// Result modifiers
    pub summary: Option<SummaryMode>,
    pub elements: Option<Vec<String>>,
    /// Include directives
    pub include: Vec<IncludeSpec>,
    pub revinclude: Vec<IncludeSpec>,
}

/// Base search capability for a storage backend.
#[async_trait]
pub trait SearchProvider: ResourceStorage {
    /// Executes a search query against a resource type within a tenant's scope.
    async fn search(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        query: &SearchQuery,
    ) -> Result<SearchBundle, StorageError>;
    
    
}

Advanced Search Capabilities as Extension Traits
Not every storage backend can support every search feature. A relational database might handle token searches efficiently but struggle with subsumption queries that require terminology reasoning. A vector database might excel at text search but lack native support for date range queries. We model these variations as extension traits.
Chained Search Provider:
/// Adds support for chained parameter searches.
/// 
/// Chaining allows searching by properties of referenced resources,
/// e.g., `Observation?patient.name=Smith`. This typically requires
/// join operations or graph traversal and must respect tenant boundaries
/// when following references.
#[async_trait]
pub trait ChainedSearchProvider: SearchProvider {
    /// Executes a search with chained parameters within a tenant's scope.
    /// 
    /// The implementation must ensure that chained references do not
    /// cross tenant boundaries except for shared resources.
    async fn search_with_chain(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        chain: &ChainedParameter,
        terminal_condition: &SearchCondition,
    ) -> Result<SearchBundle, StorageError>;

    /// Executes a reverse chain (_has) search within a tenant's scope.
    /// 
    /// Finds resources that are referenced by other resources matching
    /// the given criteria, respecting tenant isolation.
    async fn search_reverse_chain(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        has_param: &HasParameter,
    ) -> Result<SearchBundle, StorageError>;

    /// Returns the maximum chain depth supported by this backend.
    fn max_chain_depth(&self) -> usize {
        4 // Reasonable default; deep chains are expensive
    }
}

/// Represents a chained parameter like `patient.organization.name`
pub struct ChainedParameter {
    /// The chain of reference parameters to follow
    pub chain: Vec<ChainLink>,
}

pub struct ChainLink {
    /// The search parameter name (must be a reference type)
    pub parameter: String,
    /// Optional type restriction for polymorphic references
    pub target_type: Option<String>,
}

/// Represents a _has parameter for reverse chaining
pub struct HasParameter {
    /// The resource type that references us
    pub referencing_type: String,
    /// The reference parameter on that type pointing to us
    pub reference_param: String,
    /// The condition to apply to the referencing resource
    pub condition: SearchCondition,
    /// Nested _has for multi-level reverse chains
    pub nested: Option<Box<HasParameter>>,
}

Terminology Search Provider:
/// Adds terminology-aware search capabilities.
/// 
/// Supports the `:above`, `:below`, `:in`, and `:not-in` modifiers
/// which require understanding of code system hierarchies and value set
/// membership. Terminology resources are typically shared across tenants,
/// but the search itself is tenant-scoped.
#[async_trait]
pub trait TerminologySearchProvider: SearchProvider {
    /// Expands a code using `:below` semantics (descendants) within tenant scope.
    /// 
    /// Returns all codes subsumed by the given code. These codes are then
    /// used to filter resources belonging to the specified tenant.
    async fn expand_below(
        &self,
        tenant: &TenantContext,
        system: &str,
        code: &str,
    ) -> Result<Vec<ExpandedCode>, StorageError>;

    /// Expands a code using `:above` semantics (ancestors) within tenant scope.
    async fn expand_above(
        &self,
        tenant: &TenantContext,
        system: &str,
        code: &str,
    ) -> Result<Vec<ExpandedCode>, StorageError>;

    /// Checks value set membership for `:in` modifier within tenant scope.
    /// 
    /// The value set itself may be shared or tenant-specific; the implementation
    /// must resolve the correct value set based on tenant context.
    async fn check_membership(
        &self,
        tenant: &TenantContext,
        valueset_url: &str,
        system: &str,
        code: &str,
    ) -> Result<bool, StorageError>;

    /// Expands a value set to all member codes within tenant scope.
    /// 
    /// Used for `:in` searches when the backend can efficiently filter
    /// by an expanded code list.
    async fn expand_valueset(
        &self,
        tenant: &TenantContext,
        valueset_url: &str,
    ) -> Result<Vec<ExpandedCode>, StorageError>;
}

pub struct ExpandedCode {
    pub system: String,
    pub code: String,
    pub display: Option<String>,
}

Text Search Provider:
/// Adds full-text search capabilities.
/// 
/// Supports `_text` (narrative search) and `_content` (full resource search)
/// parameters, as well as the `:text` modifier on string parameters.
/// All searches are scoped to the specified tenant.
#[async_trait]
pub trait TextSearchProvider: SearchProvider {
    /// Searches resource narratives within a tenant's scope.
    /// 
    /// Matches against the XHTML content in `Resource.text.div`.
    async fn search_text(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        text_query: &str,
        additional_params: &SearchQuery,
    ) -> Result<SearchBundle, StorageError>;

    /// Searches full resource content within a tenant's scope.
    /// 
    /// Matches against all string content in the resource JSON.
    async fn search_content(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        content_query: &str,
        additional_params: &SearchQuery,
    ) -> Result<SearchBundle, StorageError>;

    /// Executes a text search on a specific parameter within tenant scope.
    /// 
    /// Used for the `:text` modifier on string and token parameters,
    /// e.g., `Condition?code:text=heart attack`.
    async fn search_parameter_text(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        parameter: &str,
        text_query: &str,
    ) -> Result<SearchBundle, StorageError>;
}

This decomposition has practical consequences. When configuring a polyglot persistence layer, we can route terminology-aware searches to a backend that integrates with a terminology server (perhaps backed by a graph database), while directing simple token matches to a faster document store. The trait system makes these routing decisions explicit and type-safe.
Transactions: When Atomicity Matters
FHIR defines batch and transaction bundles. A batch processes entries independently; a transaction either succeeds completely or fails entirely with no partial effects. This all-or-nothing semantics requires database-level transaction support - something not all storage technologies provide natively.
/// Provides ACID transaction support.
/// 
/// Transactions group multiple operations into an atomic unit. All
/// operations within a transaction are tenant-scoped; a single transaction
/// cannot span multiple tenants.
#[async_trait]
pub trait TransactionProvider: ResourceStorage {
    /// Begins a new transaction within tenant scope.
    /// 
    /// All operations on the returned Transaction object are scoped
    /// to the specified tenant and will be committed or rolled back
    /// as a unit.
    async fn begin_transaction(
        &self,
        tenant: &TenantContext,
    ) -> Result<Box<dyn Transaction>, StorageError>;
}

/// An active transaction.
/// 
/// Operations within a transaction see their own uncommitted changes
/// but are isolated from concurrent transactions.
#[async_trait]
pub trait Transaction: Send + Sync {
    /// Returns the tenant context for this transaction.
    fn tenant(&self) -> &TenantContext;

    /// Creates a resource within this transaction.
    async fn create(&mut self, resource: &Value) -> Result<StoredResource, StorageError>;

    /// Reads a resource within this transaction (sees uncommitted changes).
    async fn read(
        &self,
        resource_type: &str,
        id: &str,
    ) -> Result<Option<StoredResource>, StorageError>;

    /// Updates a resource within this transaction.
    async fn update(&mut self, resource: &Value) -> Result<StoredResource, StorageError>;

    /// Deletes a resource within this transaction.
    async fn delete(&mut self, resource_type: &str, id: &str) -> Result<(), StorageError>;

    /// Commits all operations in this transaction atomically.
    async fn commit(self: Box<Self>) -> Result<(), StorageError>;

    /// Rolls back all operations in this transaction.
    async fn rollback(self: Box<Self>) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub enum IsolationLevel {
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

A storage backend that doesn't support transactions can still handle batch operations. It simply processes each entry independently, accepting that failures may leave partial results. The trait separation makes this distinction clear: code that requires atomicity takes &dyn TransactionProvider, while code that can tolerate partial failures takes &dyn ResourceStorage.
Audit Events: A Separated Persistence Store
AuditEvent resources should be ideally stored separately from clinical data. This isn't just a security concern, it's also an architectural one. Audit logs have different access patterns (append-heavy, rarely queried except during investigations), different retention requirements (often longer than clinical data), and different security constraints (must be tamper-evident, may require separate access controls).
/// Specialized storage for audit events.
/// 
/// Audit storage is intentionally separate from clinical data storage.
/// It typically has different characteristics:
/// - Append-only or append-heavy workload
/// - Different retention policies
/// - Tamper-evident storage requirements
/// - Separate access control
#[async_trait]
pub trait AuditStorage: Send + Sync {
    /// Records an audit event. This operation should be highly available
    /// and should not fail clinical operations if audit storage is degraded.
    async fn record(&self, tenant: &TenantContext, event: &AuditEvent) -> Result<String, AuditError>;

    /// Queries audit events within a time range.
    async fn query(
        &self,
        criteria: &AuditQuery,
    ) -> Result<Vec<AuditEvent>, AuditError>;

    /// Retrieves audit events for a specific resource (accounting of disclosures).
    async fn disclosures_for_resource(
        &self,
        resource_type: &str,
        resource_id: &str,
        period: &DateRange,
    ) -> Result<Vec<AuditEvent>, AuditError>;
}

/// Audit query criteria supporting HIPAA accounting requirements.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub patient_id: Option<String>,
    pub agent_id: Option<String>,
    pub action: Option<AuditAction>,
    pub period: Option<DateRange>,
    pub resource_type: Option<String>,
    pub outcome: Option<AuditOutcome>,
}

The separation of AuditStorage from ResourceStorage enables critical architectural flexibility. Audit events can flow to a dedicated time-series database optimized for append-only writes, or to an immutable ledger for tamper evidence, or to a separate cloud account for security isolation.
The REST Layer: Mapping HTTP to Storage
The FHIR REST API defines interactions (read, vread, update, create, delete, search, etc.) that map HTTP verbs and URL patterns to operations. This mapping is a separate concern from storage. The same storage backend might be accessed via REST, GraphQL, messaging, or bulk export.
/// Interaction types defined by the FHIR REST specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    Read,
    Vread,
    Update,
    Patch,
    Delete,
    History,
    Create,
    Search,
    Capabilities,
    Batch,
    Transaction,
}

/// Scope at which an interaction operates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionScope {
    Instance,   // Operations on a specific resource instance
    Type,       // Operations on a resource type
    System,     // System-wide operations
}

/// Result of a REST interaction, capturing both outcome and metadata.
pub struct InteractionResult {
    pub resource: Option<StoredResource>,
    pub status: HttpStatus,
    pub etag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub location: Option<String>,
    pub outcome: Option<OperationOutcome>,
}

/// Orchestrates REST interactions by coordinating storage traits.
#[async_trait]
pub trait RestHandler: Send + Sync {
    /// Processes a FHIR REST interaction.
    async fn handle(
        &self,
        interaction: Interaction,
        scope: InteractionScope,
        context: &InteractionContext,
    ) -> Result<InteractionResult, RestError>;
}

The RestHandler is a coordination layer that combines multiple storage traits to implement FHIR REST semantics. A read interaction needs only ResourceStorage. A vread needs VersionedStorage. A search with _include needs both SearchProvider and ResourceStorage. The REST handler composes these capabilities based on what the request requires and what the storage backend provides.
Capability Statements: Documenting What Storage Supports
The FHIR specification requires servers to publish a CapabilityStatement declaring which interactions, resources, and search parameters they support. When storage backends have different capabilities, this statement must accurately reflect the union of what's available and identify gaps.
Diesel solves a similar problem with its type system. Operations that aren't supported simply don't compile. For FHIR, we need runtime capability discovery because queries are dynamic. We model storage capabilities as a queryable trait that can generate CapabilityStatement fragments:
/// Declares the capabilities of a storage backend.
/// Inspired by Diesel's approach to backend-specific features.
pub trait StorageCapabilities {
    /// Returns supported interactions for a resource type.
    fn supported_interactions(&self, resource_type: &str) -> Vec<Interaction>;

    /// Returns supported search parameters for a resource type.
    fn supported_search_params(&self, resource_type: &str) -> Vec<SearchParamCapability>;

    /// Returns supported search modifiers for a parameter type.
    fn supported_modifiers(&self, param_type: SearchParamType) -> Vec<SearchModifier>;

    /// Returns whether chained search is supported.
    fn supports_chaining(&self) -> bool;

    /// Returns whether reverse chaining (_has) is supported.
    fn supports_reverse_chaining(&self) -> bool;

    /// Returns whether _include is supported.
    fn supports_include(&self) -> bool;

    /// Returns whether _revinclude is supported.
    fn supports_revinclude(&self) -> bool;

    /// Returns supported transaction isolation levels.
    fn supported_isolation_levels(&self) -> Vec<IsolationLevel>;

    /// Generates a FHIR CapabilityStatement fragment for this backend.
    fn to_capability_statement(&self) -> Value;
}

/// Describes support for a specific search parameter.
#[derive(Debug, Clone)]
pub struct SearchParamCapability {
    pub name: String,
    pub param_type: SearchParamType,
    pub modifiers: Vec<SearchModifier>,
    pub prefixes: Vec<SearchPrefix>,
    pub documentation: Option<String>,
}

/// Marker traits for optional capabilities, enabling compile-time 
/// capability checking where possible (similar to Diesel's backend features).
pub trait SupportsExactMatch: SearchBackend {}
pub trait SupportsContainsMatch: SearchBackend {}
pub trait SupportsTerminologySearch: SearchBackend {}
pub trait SupportsFullTextSearch: SearchBackend {}
pub trait SupportsChainedSearch: SearchBackend {}
pub trait SupportsReverseChaining: SearchBackend {}

Dynamic Capability Checking
For operations that can't be checked at compile time, we provide runtime capability checking that fails fast with clear error messages:
/// Validates that a search query can be executed by this backend.
pub trait QueryValidator: StorageCapabilities {
    /// Checks if all features required by the query are supported.
    fn validate_query(&self, query: &SearchQuery) -> Result<(), UnsupportedFeature>;
    
    /// Returns which parts of a query would need post-processing.
    fn requires_post_processing(&self, query: &SearchQuery) -> Vec<PostProcessingStep>;
}

#[derive(Debug)]
pub struct UnsupportedFeature {
    pub feature: String,
    pub parameter: Option<String>,
    pub suggestion: Option<String>,
}

impl std::fmt::Display for UnsupportedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unsupported feature: {}", self.feature)?;
        if let Some(ref param) = self.parameter {
            write!(f, " on parameter '{}'", param)?;
        }
        if let Some(ref suggestion) = self.suggestion {
            write!(f, ". Suggestion: {}", suggestion)?;
        }
        Ok(())
    }
}

The Feature Support Matrix
Different storage technologies have different strengths. A key deliverable of the Helios FHIR Server's persistence design is a clear feature support matrix that documents what each storage backend provides. This (example, work-in-progress) matrix drives both the CapabilityStatement generation and helps operators choose the right backend for their workload.
Feature	PostgreSQL	MongoDB	Cassandra	Neo4j	Elasticsearch	S3/Parquet
Basic CRUD	✓	✓	✓	✓	✓	Read-only
Versioning	✓	✓	✓	✓	✓	✓
History	✓	✓	Limited	✓	✓	✓
Transactions	✓	✓	Limited	✓	✗	✗
Multitenancy	RLS + App	Collection/DB	Keyspace	Labels	Index per tenant	Prefix-based
Search: String	✓	✓	Limited	✓	✓	✓
Search: Token	✓	✓	✓	✓	✓	✓
Search: Reference	✓	✓	Limited	✓	✓	✓
Search: Date Range	✓	✓	✓	✓	✓	✓
Search: Quantity	✓	✓	Limited	Limited	✓	✓
Modifier: :exact	✓	✓	✓	✓	✓	✓
Modifier: :contains	✓	✓	✗	✓	✓	✗
Modifier: :not	✓	✓	✗	✓	✓	✓
Modifier: :missing	✓	✓	Limited	✓	✓	✓
Modifier: :above/:below	With terminology	With terminology	✗	✓	With terminology	✗
Modifier: :in/:not-in	With terminology	With terminology	✗	✓	With terminology	✗
Chained Parameters	✓	✓	✗	✓	Limited	✗
Reverse Chaining (_has)	✓	✓	✗	✓	✗	✗
_include	✓	✓	✗	✓	✓	✗
_revinclude	✓	✓	✗	✓	✓	✗
Full-text Search	✓	✓	✗	✗	✓	✗
Bulk Export	✓	✓	✓	✓	✓	✓
This matrix isn't static. It's generated from the StorageCapabilities implementations. When a new storage backend is added or an existing one gains features, the matrix updates automatically.
Composing Storage Backends (Inspired by Diesel's MultiConnection)
Diesel's MultiConnection derive macro generates an enum that wraps multiple connection types, dispatching to the appropriate backend at runtime. We adapt this pattern for polyglot FHIR persistence, but with intelligent routing based on query characteristics:
/// Routes operations to appropriate storage backends based on capabilities
/// and query characteristics. Similar to Diesel's MultiConnection but with
/// query-aware routing.
pub struct CompositeStorage {
    /// Primary transactional store for CRUD operations
    primary: Arc<dyn ResourceStorage>,
    
    /// Search-optimized store (may be the same as primary)
    search: Arc<dyn SearchProvider>,
    
    /// Terminology service for subsumption queries
    terminology: Arc<dyn TerminologySearchProvider>,
    
    /// Graph store for relationship traversal
    graph: Option<Arc<dyn ChainedSearchProvider>>,
    
    /// Full-text search engine
    text: Option<Arc<dyn TextSearchProvider>>,
    
    /// Audit log store (always separate)
    audit: Arc<dyn AuditStorage>,
    
    /// Bulk export store
    bulk: Arc<dyn BulkExportProvider>,
    
    /// Query cost estimator for routing decisions
    cost_estimator: Arc<dyn QueryCostEstimator>,
}

The routing logic becomes explicit policy that considers both capabilities and cost:
impl CompositeStorage {
    async fn route_search(
        &self, 
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> Result<SearchBundle, StorageError> {
        // If query contains _text or _content, route to text search
        if query.has_text_search() {
            if let Some(ref text) = self.text {
                return text.search(tenant, &query.resource_type, query).await;
            }
            return Err(StorageError::UnsupportedFeature(UnsupportedFeature {
                feature: "full-text search".into(),
                parameter: query.text_param_name(),
                suggestion: Some("Remove _text/_content parameters or enable Elasticsearch backend".into()),
            }));
        }

        // If query contains :above or :below modifiers, involve terminology
        if query.has_terminology_modifiers() {
            return self.search_with_terminology(tenant, query).await;
        }

        // If query contains chained parameters, prefer graph store
        if query.has_chaining() {
            if let Some(ref graph) = self.graph {
                let graph_cost = self.cost_estimator.estimate_cost(query, graph.as_ref());
                let primary_cost = self.cost_estimator.estimate_cost(query, self.search.as_ref());
                
                if graph_cost < primary_cost {
                    return graph.search(tenant, &query.resource_type, query).await;
                }
            }
        }

        // Default to primary search
        self.search.search(tenant, &query.resource_type, query).await
    }
    
    /// Ensures _include and _revinclude respect tenant boundaries.
    async fn apply_includes(
        &self,
        tenant: &TenantContext,
        matches: Vec<StoredResource>,
        query: &SearchQuery,
    ) -> Result<SearchBundle, StorageError> {
        let mut included = Vec::new();
        
        // Process _include directives
        for include in &query.include {
            let resolved = self.search.resolve_includes(tenant, &matches, &[include.clone()]).await?;
            included.extend(resolved);
        }
        
        // Process _revinclude directives  
        for revinclude in &query.revinclude {
            let resolved = self.search.resolve_revincludes(tenant, &matches, &[revinclude.clone()]).await?;
            included.extend(resolved);
        }
        
        Ok(SearchBundle {
            matches,
            included,
            total: None,
        })
    }
}

/// Estimates the cost of executing a query on different backends.
pub trait QueryCostEstimator: Send + Sync {
    fn estimate_cost(
        &self,
        query: &SearchQuery,
        backend: &dyn StorageCapabilities,
    ) -> QueryCost;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum QueryCost {
    /// Query can be executed efficiently
    Optimal(u64),
    /// Query requires some post-processing
    Acceptable(u64),
    /// Query will be slow, consider alternatives
    Expensive(u64),
    /// Query requires expanding a code set first
    RequiresExpansion,
    /// Backend cannot execute this query
    Unsupported,
}

The Path Forward
This trait-based decomposition provides a foundation for building a FHIR persistence layer that can evolve with requirements. When AI workloads demand vector similarity search, we add a VectorSearchProvider trait and plug in a vector database. When regulatory requirements demand immutable audit trails, we implement AuditStorage against an append-only ledger. When performance analysis reveals that graph traversals are bottlenecking population health queries, we route those operations to a dedicated graph database.
Extensibility Following Diesel's Model: Just as Diesel's sql_function! macro makes it trivial to add custom SQL functions, our design should make it easy to add custom search parameters and modifiers. A healthcare organization might need a custom :phonetic modifier for patient name matching, or a :geo-near modifier for location-based searches. The SearchFragment pattern enables this:
// Adding a custom phonetic search modifier is straightforward
pub struct PhoneticMatch {
    pub parameter: String,
    pub path: FhirPath,
    pub value: String,
    pub algorithm: PhoneticAlgorithm,
}

impl<B: SearchBackend> SearchFragment<B> for PhoneticMatch 
where
    B: SupportsPhoneticSearch,  // Custom capability marker
{
    fn apply(&self, builder: &mut B::QueryBuilder) -> Result<(), SearchError> {
        builder.add_phonetic_match(&self.path, &self.value, &self.algorithm)
    }
    
    fn is_native(&self, backend: &B) -> bool {
        backend.has_phonetic_support(&self.algorithm)
    }
    
    fn estimated_cost(&self, backend: &B) -> QueryCost {
        backend.cost_for_phonetic(&self.path)
    }
}

This is what it means to build FHIR persistence for the AI era: not a monolithic database adapter, but a composable system of specialized capabilities that can be assembled to meet the specific needs of each deployment with tenant isolation, search routing, and extensibility built into the architecture from the start.
Thank you!
I very much look forward to your thoughts on these ideas and to the discussions that follow.
Sincerely, -Steve
