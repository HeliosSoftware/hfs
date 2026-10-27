# Resources create targets

The Resources workspace keeps search and creation separate. A type can remain
in the rail and query builder even when the UI cannot create it.

Create is enabled only when the type passes all three checks for the request's
effective FHIR version:

1. The compiled resource catalog contains the exact, case-sensitive type.
2. The live, tenant-scoped `CapabilityStatement` advertises a `create`
   interaction for that type.
3. The editor's core schema registry resolves the type to a resource schema.

The UI fetches `/metadata` through its authenticated loopback client. If that
request fails, creation stays disabled. Unknown types, types from another FHIR
version, read-only resources, and types without an editor schema also stay
disabled. The page keeps the typed query visible so the user can correct it.
The `url` query parameter supplies the effective type when both `url` and
`type` are present.

This rule uses the compiled core schema pack on purpose. HFS can include
built-in extensions such as `ViewDefinition` in its compiled resource catalog;
they are eligible when the same binary also has their editor schema and
advertises `create`. Tenant-defined additional resources are different. FHIR
R6 allows a server to advertise them through
`CapabilityStatement.rest.resource.definition` and a `StructureDefinition`,
but the current editor does not load tenant-defined resource schemas. Those
resources may be searchable, but the Resources workspace will not create them
until it can resolve the matching runtime schema.

## Resource names and path-based tenants

HFS reserves every resource name from every FHIR version compiled into the
binary, using a case-insensitive comparison for the first URL path segment.
This keeps a resource path from being mistaken for a tenant when the selected
FHIR version does not contain that resource. It also means a path-based tenant
whose id matches any compiled resource name can no longer be addressed through
`/{tenant}/...`.

Operators with such a tenant can select it through the `X-Tenant-ID` header and
use ordinary FHIR resource paths, or migrate its data to a tenant id that does
not collide with a compiled resource name. HFS does not rename tenant data
automatically.

`ViewDefinition` remains HFS's compiled extension to the resource set.
Its advertised definition and version alignment need a separate decision if
HFS moves it to the same runtime-definition model as R6 additional resources.
