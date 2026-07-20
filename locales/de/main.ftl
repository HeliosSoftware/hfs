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
nav-search-parameters = Suchparameter
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

## SearchParameter-Ansicht (#238)

sp-heading = Suchparameter
sp-lede = Durchsuche die Parameter, mit denen dieser Server Suchen auflöst, gefiltert nach Basis-Ressourcentyp. Spezifikationsparameter sind schreibgeschützt; tenant-spezifisches Bearbeiten kommt, sobald Suchparameter im Storage liegen.
sp-version-label = FHIR-Version
sp-spec-missing = Das vollständige Spezifikations-Bundle (search-parameters-*.json) wurde im Datenverzeichnis nicht gefunden — es werden nur die minimalen eingebetteten Fallback-Parameter angezeigt.
sp-rail-label = Ressourcenfilter
sp-rail-search = Typen filtern
sp-rail-recent = Zuletzt verwendet
sp-rail-types = Ressourcentypen
sp-rail-all = Alle Typen
sp-facet-type = Typ
sp-facet-type-label = Nach Parametertyp filtern
sp-facet-source = Quelle
sp-facet-source-label = Nach Quelle filtern
sp-source-embedded = eingebettet
sp-source-stored = gespeichert
sp-source-config = Konfiguration
sp-chip-conflict = Konflikt
sp-chip-overrides = überschreibt Spez.
sp-chip-shadowed = verdeckt
sp-col-code = Code
sp-col-type = Typ
sp-col-base = Basis
sp-col-expression = Ausdruck
sp-col-source = Quelle
sp-total = { $count } Parameter
sp-pagination-label = Seiten
sp-page-prev = Zurück
sp-page-next = Weiter
sp-detail-label = Parameterdetails
sp-detail-empty = Kein Parameter ausgewählt
sp-detail-empty-hint = Wähle eine Zeile, um Definition, Ausdruck und die Auflösung im Register zu prüfen.
sp-detail-readonly = Spezifikationsparameter (aus der Datendatei einkompiliert) — schreibgeschützt.
sp-field-url = Kanonische URL
sp-field-name = Name
sp-field-status = Status
sp-field-base = Basis-Ressourcentypen
sp-field-expression = FHIRPath-Ausdruck
sp-field-description = Beschreibung
sp-field-target = Zieltypen
sp-field-components = Komponenten
sp-status-hint = Der Loader stuft den Draft-Status der Spezifikation beim Laden auf active hoch.
sp-note-conflict = Doppeltes (base, code) innerhalb derselben Quelle wie { $url } — das Register lehnt diese Kollision ab (DuplicateCode).
sp-note-overrides = Überschreibt { $url } auf (base, code): eine gespeicherte Definition hat Vorrang vor dem Spezifikationsparameter und löst daher die Suchen auf. Das Register loggt ein WARN mit beiden URLs.
sp-note-shadowed = Verdeckt durch { $url } auf (base, code): eine Quelle mit höherem Vorrang löst die Suchen für diesen Slot auf.
sp-note-empty-expression = Leerer Ausdruck: der Extractor indexiert keine Zeilen, jede Suche über diesen Parameter liefert stillschweigend nichts.
sp-note-no-target = Referenzparameter ohne Zieltypen: verkettete Suche kann den referenzierten Typ nicht auflösen.
sp-note-choice-type = Choice-Typ-Ausdruck: der Extractor schreibt ofType(T) / as T vor der Auswertung gegen das gespeicherte JSON auf das konkrete Element um (z. B. valueQuantity).
sp-writes-pending = Anlegen, Überschreiben und Löschen von Tenant-Parametern kommt, sobald Suchparameter in der Datenbank gespeichert werden (#235).

## Compartment-Ansicht & Tester (#237)

cmp-heading = Compartments
cmp-lede = Die Compartment-Definitionen, mit denen dieser Server /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"}-Anfragen routet, und ein Tester, der beantwortet: Ist dieser Typ in diesem Compartment, über welche Parameter, und welche Suche führt der Server aus?
cmp-rail-label = Compartment-Definitionen
cmp-rail-heading = Compartments
cmp-rail-note = Die Basisdefinitionen werden mit dem Server ausgeliefert (aus der FHIR-Spezifikation generiert). Sie zu bearbeiten setzt eine tenant-spezifische Override-Schicht voraus — offene Frage im Issue.
cmp-tabs-label = Compartment-Bereiche
cmp-tab-definition = Definition
cmp-tab-members = Mitglieder
cmp-tab-tester = Tester
cmp-field-code = Code
cmp-field-status = Status
cmp-field-url = Kanonische URL
cmp-field-version = Version
cmp-field-publisher = Herausgeber
cmp-field-description = Beschreibung
cmp-field-search = search
cmp-field-experimental = experimental
cmp-search-why = Aus würde bedeuten, dass keine Compartment-Route für dieses Compartment auflöst.
cmp-on = an
cmp-off = aus
cmp-yes = ja
cmp-no = nein
cmp-readonly-note = Schreibgeschützt: diese Werte stammen aus den in den Server einkompilierten Spezifikationsdefinitionen.
cmp-filter-members = Mitglieder
cmp-filter-all = Alle Typen
cmp-filter-excluded = Ausgeschlossen
cmp-member = Mitglied
cmp-excluded = ausgeschlossen
cmp-tester-id = Id
cmp-tester-target = Zieltyp (oder *)
cmp-tester-run = Testen
cmp-result-member = ✓ Mitglied — über { $params }
cmp-result-flat = // äquivalente flache Suche
cmp-result-member-note = Der Server löst die Compartment-Route zu dieser Suche über die Referenzparameter des Typs auf.
cmp-result-self = ✓ Mitglied — die Compartment-Ressource selbst ({"{"}def{"}"})
cmp-result-self-note = Die Compartment-Instanz ist trivialerweise in ihrem eigenen Compartment; die Route liest die Ressource direkt.
cmp-result-notmember = ✕ { $type } ist kein Mitglied dieses Compartments
cmp-result-notmember-note = Der Server antwortet mit 404 und einem OperationOutcome für Typen, die keine Compartment-Mitglieder sind.
cmp-result-fanout = Fächert auf { $count } Mitgliedstypen auf
cmp-result-fanout-note = Ausgeschlossene Typen werden übersprungen, nicht fehlgeschlagen — der Fan-out lässt Nicht-Mitgliedstypen weg statt zu scheitern.
queries-builder-heading = Such-Builder
queries-url-label = FHIR-Such-URL
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Bearbeite die GET-URL direkt oder über die Zeilen darunter — beide bleiben synchron. Ausführen führt die Suche hier aus und trägt sie unter „Zuletzt" ein; mit einem Namen bleibt sie in der Liste gespeichert.
queries-recent = Zuletzt
queries-recent-heading = Letzte Suchen
queries-recent-empty = Noch keine letzten Suchen — führe eine aus, um sie hier einzutragen.
queries-invalid-url = Gib eine Suche wie GET /Patient?name=smith ein — der Ressourcentyp kommt aus dem Pfad.

queries-conditions = Bedingungen
queries-add-condition = Bedingung hinzufügen
queries-includes = Includes
queries-result-controls = Ergebnis-Steuerung
queries-remove = Entfernen
queries-param-placeholder = Parameter
queries-value-placeholder = Wert
queries-results = Ergebnisse
queries-results-total = { $count } Ergebnisse
queries-results-included = { $count } eingeschlossen
queries-results-empty = Keine Ergebnisse.
queries-open-tab = In neuem Tab öffnen
queries-col-updated = Aktualisiert
queries-prev = Zurück
queries-next = Weiter

queries-rail-heading = Ressourcentypen
queries-rail-filter = Typen filtern

## Suche — natürliche Sprache & visueller Builder (#255)

search-heading = Suche
search-lede = Beschreiben Sie, wonach Sie suchen, oder bauen Sie die Abfrage selbst. So oder so erhalten Sie eine FHIR-Suchabfrage, die Sie lesen, korrigieren und ausführen können.
search-query-tag = ABFRAGE
search-copy = Abfrage kopieren

search-mode-label = Wie die Abfrage entsteht
search-mode-nl = Natürliche Sprache
search-mode-builder = Visueller Builder

search-nl-label = Suche beschreiben
search-nl-placeholder = Beschreiben Sie, wonach Sie suchen — z. B. Patienten namens Smith, geboren nach 1980
search-nl-hint = Ihr Text und die Suchparameter dieses Servers gehen an das Sprachmodell. Patientendaten niemals. Die erzeugte Abfrage wird unten angezeigt — zum Prüfen und Ausführen.
search-nl-working = Wird übersetzt…
search-nl-caveats = Wichtig zu wissen:
search-nl-unsupported = Das ist keine Suche, die dieser Server ausführen kann. Beschreiben Sie die Datensätze, die Sie finden möchten.

search-nl-example-1 = Weibliche Patientinnen über 65 mit Diabetes-Diagnose
search-nl-example-2 = Beobachtungen der letzten 30 Tage, neueste zuerst
search-nl-example-3 = Laufende Fälle im Boston General

search-setup-heading = Suche in natürlicher Sprache ist verfügbar
search-setup-body = Verwandelt Beschreibungen in Alltagssprache in FHIR-Suchabfragen. Dafür wird ein API-Schlüssel für ein Sprachmodell benötigt — der Server liest ihn aus der Umgebung, und er gelangt nie auf diese Seite. Bis einer gesetzt ist, nutzen Sie den visuellen Builder unten.
search-setup-key-placeholder = Ihr API-Schlüssel
search-setup-disable = Um die Funktion vollständig zu entfernen — Endpunkt, Seite und diesen Hinweis — setzen Sie HFS_NL_SEARCH_ENABLED=false.
search-setup-docs = Anleitung lesen
