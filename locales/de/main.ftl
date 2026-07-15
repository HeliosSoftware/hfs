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
nav-tenants = Mandanten
nav-toggle = Navigation ein- oder ausklappen

## Mandantenverwaltung (/ui/tenants)

tenants-title = Mandantenverwaltung
tenants-unavailable = Die Mandantenregistrierung ist auf diesem Speicher-Backend nicht verfügbar.
tenants-stat-total = Mandanten gesamt
tenants-stat-total-sub = { $count ->
    [one] { $count } registriert
   *[other] { $count } registriert
}
tenants-stat-resources = Gespeicherte Ressourcen
tenants-stat-resources-sub = über alle Mandanten
tenants-search-placeholder = Nach Name oder Mandanten-ID suchen…
tenants-add = Mandant hinzufügen
tenants-add-title = Einen Mandanten hinzufügen
tenants-field-id = Mandanten-ID
tenants-field-id-hint = Wird in der API verwendet (Header X-Tenant-ID, URL-Präfix, JWT-Claim).
tenants-field-name = Anzeigename (optional)
tenants-field-name-hint = Eine lesbare Bezeichnung; nicht für das Routing verwendet.
tenants-add-submit = Mandant bereitstellen
tenants-col-tenant = Mandant
tenants-col-resources = Ressourcen
tenants-col-created = Erstellt
tenants-col-actions = Aktionen
tenants-empty = Keine Mandanten gefunden.
tenants-unregistered = nicht registriert
tenants-delete = Mandant löschen
tenants-delete-confirm = Mandant „{ $id }" abmelden? Die gespeicherten Daten bleiben erhalten, sofern sie nicht über die API bereinigt werden.

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
chart-expand = Diagramm vergrößern
chart-window = Zeitfenster des Diagramms

## Fußzeile

footer-copyright = © { $year } { -org-name }

## Verlauf & Versionen (#236)

history-heading = Verlauf & Versionen
history-lede = Zwei Versionen einer Ressource vergleichen. Der Speicher ist vollständig versioniert; dies liest ihn über die übliche _history- und vread-API.
history-type-label = Ressourcentyp
history-id-label = Ressourcen-ID
history-id-placeholder = Ressourcen-ID
history-load = Laden
history-tabs-label = Verlaufsbereich
history-tab-instance = Instanz
history-tab-type = Typ-Feed
history-tab-system = System-Feed
history-versions-label = Versionen
history-pick-instance = Instanz wählen
history-current = aktuell
history-from = Von
history-to = Bis
history-show-metadata = Metadatenänderungen anzeigen
history-empty = Laden Sie eine Ressource und wählen Sie zwei Versionen zum Vergleich.
history-load-error = Der Verlauf dieser Ressource konnte nicht geladen werden.
history-not-found = Kein Verlauf für diese Ressource — Typ und ID prüfen.
history-diff-heading = { $from }
history-metadata-hidden = { $count ->
    [one] { $count } Metadatenänderung ausgeblendet
   *[other] { $count } Metadatenänderungen ausgeblendet
}
history-textual = Vollständigen Text-Diff anzeigen
history-only-metadata = Zwischen diesen Versionen änderten sich nur die Metadaten.
history-identical = Diese beiden Versionen sind identisch.
history-deleted = { $version } ist eine Löschung — es gibt nichts zu vergleichen.
history-parse-error = Diese Versionen konnten nicht als JSON gelesen werden.
