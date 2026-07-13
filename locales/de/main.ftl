# Helios FHIR-Server — UI-Nachrichtenkatalog
# Gebietsschema: Deutsch (de)
#
# Verwenden Sie dieselben Schlüssel wie in `en/main.ftl` (Quell-Gebietsschema).
# Fehlende Schlüssel greifen gemäß der in docs/multi-language.md beschriebenen
# Fallback-Kette auf Englisch zurück.

## Marke / gemeinsame Begriffe

-app-name = Helios FHIR-Server
-org-name = Helios Software

## Seitenstruktur

app-title = { -app-name }
app-tagline = Ein schneller, versionsübergreifender FHIR-Server

nav-dashboard = Übersicht
nav-terminology = Terminologie
nav-resources = Ressourcen
nav-settings = Einstellungen
nav-signout = Abmelden

## Sprachauswahl

language-label = Sprache
language-en = Englisch
language-es = Spanisch
language-de = Deutsch

## Startseite

home-lede = Serverseitig gerenderte, HTMX-basierte Oberfläche. Dieses Panel wird als HTML-Fragment aktualisiert.

## Statuspanel

status-last-checked = Zuletzt geprüft: { $timestamp }

## Übersicht / Status

dashboard-heading = Server-Übersicht
health-status-ok = Alle Systeme betriebsbereit
health-status-degraded = Einige Systeme sind beeinträchtigt
health-uptime = Betriebszeit: { $duration }

resource-count = { $count ->
    [one] { $count } Ressource
   *[other] { $count } Ressourcen
}

## Terminologie durchsuchen

terminology-search-label = CodeSystems und ValueSets durchsuchen
terminology-search-placeholder = z. B. 73211009, „Diabetes“, http://snomed.info/sct
terminology-display-language = Anzeigesprache
terminology-no-results = Keine passenden Konzepte gefunden.

## Allgemeine Aktionen

action-search = Suchen
action-save = Speichern
action-cancel = Abbrechen
action-retry = Erneut versuchen

## Fehler (spiegelt den OperationOutcome-Text wider; siehe docs/multi-language.md §5)

error-not-found = Die angeforderte Ressource wurde nicht gefunden.
error-unauthorized = Sie sind nicht berechtigt, diese Aktion auszuführen.
error-generic = Etwas ist schiefgelaufen. Bitte versuchen Sie es erneut.

## Dashboard-Gerüst (Figma „Dashboard V1.1“)

nav-section-work = Arbeit
nav-section-batch-data = Batch & Daten
nav-section-server = Server
nav-section-conditional = Bedingt

nav-home = Startseite
nav-search = Suche
nav-resource-editor = Ressourcen-Editor
nav-history-versions = Verlauf & Versionen
nav-compartments = Compartments
nav-batch-transaction = Batch / Transaktion
nav-bulk-export = Bulk-Export
nav-sql-on-fhir = SQL-on-FHIR
nav-capability-conformance = Capability & Konformität
nav-admin-ops = Admin / Betrieb
nav-subscriptions = Abonnements

tenant-heading = Tenants
tenant-all = Alle Tenants
tenant-search-placeholder = Tenants durchsuchen

theme-label = Farbschema
theme-light = Helles Design
theme-dark = Dunkles Design

fhir-version = FHIR { $version }

card-resource-types = Ressourcentypen
card-resource-types-sub = aktiviert für { $version }
card-stored-resources = Gespeicherte Ressourcen
card-stored-resources-sub = im aktiven Tenant
card-export-jobs = Export-Jobs
card-export-jobs-sub = laufend ({ $queued } in der Warteschlange)
card-uptime = Verfügbarkeit
card-uptime-sub = letzte 30 Tage

chart-title = FHIR-Ressourcen im Zeitverlauf
chart-unit-patients = Patienten
chart-expand = Diagramm vergrößern

## Fußzeile

footer-copyright = © { $year } { -org-name }

## Saved queries (#234)

nav-saved-queries = Gespeicherte Abfragen

queries-heading = Gespeicherte Abfragen
queries-lede = FHIR-Suchabfragen je Ressourcentyp aufbewahren, sortiert nach der letzten Ausführung. Sie werden in deinen Benutzereinstellungen gespeichert und stehen auf allen Geräten bereit.
queries-add-heading = Abfrage speichern
queries-type-label = Ressourcentyp
queries-type-placeholder = z. B. Patient
queries-name-label = Name
queries-name-placeholder = z. B. Smiths in Boston
queries-query-label = Abfrage
queries-query-placeholder = z. B. name=smith&address-city=Boston
queries-empty = Noch keine gespeicherten Abfragen. Speichere oben eine, um loszulegen.
queries-never-run = Nie ausgeführt
queries-run = Ausführen
queries-rename = Umbenennen
queries-delete = Löschen
queries-rename-prompt = Neuer Name
queries-confirm-delete = „{ $name }“ löschen?
queries-unavailable = Gespeicherte Abfragen sind nicht verfügbar: Das Storage-Backend dieses Servers unterstützt keine Benutzereinstellungen.

queries-builder-heading = Such-Builder
queries-url-label = FHIR-Such-URL
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Bearbeite die GET-URL direkt. Ausführen öffnet die Suche in einem neuen Tab und trägt sie unter „Zuletzt" ein; mit einem Namen bleibt sie in der Liste unten gespeichert.
queries-recent = Zuletzt
queries-recent-heading = Letzte Suchen
queries-recent-empty = Noch keine letzten Suchen — führe eine aus, um sie hier einzutragen.
queries-invalid-url = Gib eine Suche wie GET /Patient?name=smith ein — der Ressourcentyp kommt aus dem Pfad.
