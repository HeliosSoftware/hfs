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
nav-terminology-new-window = Terminologie (wird in einem neuen Tab geöffnet)
nav-resources = Ressourcen
nav-settings = Einstellungen
nav-signout = Abmelden

## Sprachauswahl

language-label = Sprache
language-en = Englisch
language-es = Spanisch
language-de = Deutsch
user-menu-label = Kontomenü
user-anonymous = Anonymer Benutzer
user-local-hint = Authentifizierung ist deaktiviert
user-logout = Abmelden

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

terminology-heading = Terminologieserver
terminology-lede = Verbinden Sie HFS mit einem FHIR-Terminologieserver.
terminology-configured-heading = Terminologieserver konfiguriert
terminology-configured-body = HFS_TERMINOLOGY_SERVER verweist auf eine gültige Server-URL.
terminology-configured-open = Terminologieserver öffnen
terminology-invalid-heading = HFS_TERMINOLOGY_SERVER ist ungültig.
terminology-invalid-body = Verwenden Sie eine absolute HTTP- oder HTTPS-URL mit einem Host. Pfade und ein abschließender Schrägstrich sind zulässig. Fügen Sie keine Zugangsdaten, Abfrageparameter oder Fragmente ein.
terminology-invalid-note = Aktualisieren Sie die Umgebungsvariable und starten Sie HFS neu.
terminology-setup-heading = Terminologieserver verbinden
terminology-setup-body = Setzen Sie HFS_TERMINOLOGY_SERVER auf die Basis-URL des FHIR-Terminologieservers, den HFS verwenden soll.
terminology-setup-note = Setzen Sie die Variable in der Umgebung, die HFS startet, und starten Sie den Server danach neu.
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
error-generic = Etwas ist schiefgelaufen. Versuchen Sie es erneut.

## Dashboard-Gerüst (Figma "Dashboard V1.1")

nav-section-work = Arbeit
nav-section-batch-data = Batch & Daten
nav-section-sql-on-fhir = SQL on FHIR
nav-section-server = Server
nav-section-tools = Werkzeuge

nav-home = Startseite
nav-search = Suche
nav-resource-editor = Ressourcen-Editor
nav-history-versions = Verlauf & Versionen
nav-compartments = Compartments
nav-batch-transaction = Batch / Transaktion
nav-import = Importieren
nav-export = Exportieren
nav-sql-view-definitions = View-Definitionen
nav-sql-queries = SQL-Abfragen
nav-sql-views = SQL-Views
nav-sql-export = SQL-Export
nav-sql-files = Dateien
nav-capability-conformance = Capability & Konformität
nav-search-parameters = Suchparameter
nav-subscriptions = Abonnements
nav-tenants = Mandanten

## Mandantenverwaltung (/ui/tenants)

tenants-title = Mandantenverwaltung
tenants-lede = Tenants anlegen, prüfen und löschen, zwischen denen dieser Server Daten isoliert.
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
tenants-row-provisioning = Wird bereitgestellt … das kann einen Moment dauern.
tenants-row-failed = Der Mandant konnte nicht bereitgestellt werden.
tenants-dismiss = Verwerfen

tenant-heading = Tenants
tenant-all = Alle Tenants
tenant-search-placeholder = Tenants durchsuchen

theme-label = Farbschema
theme-light = Helles Design
theme-dark = Dunkles Design

fhir-version = FHIR { $version }
fhir-version-heading = FHIR-Version

card-resource-types = Ressourcentypen
card-resource-types-sub = verwendet für { $version }
card-stored-resources = Gespeicherte Ressourcen
card-stored-resources-sub = im aktiven Tenant
card-export-jobs = Export-Jobs
card-export-jobs-sub = laufend ({ $queued } in der Warteschlange)
card-import-jobs = Import-Jobs
card-import-jobs-sub = aktiv
card-jobs-unavailable = nicht verfügbar
card-uptime = Betriebszeit
card-uptime-sub = seit Prozessstart

chart-title = FHIR-Ressourcen im Zeitverlauf
chart-window = Zeitfenster des Diagramms
chart-pick-heading = Dargestellte Ressourcentypen
chart-pick-all = Alle Ressourcentypen anzeigen
chart-pick-filter = Typen filtern
chart-empty = Noch nichts darzustellen — gespeicherte Ressourcen erscheinen hier, sobald sie angelegt werden.
chart-sample-note = Beispieldaten: in diesem Build ist kein Live-Metrikanbieter registriert.
chart-table-toggle = Als Tabelle anzeigen
chart-table-when = Zeitpunkt
chart-focus-series = Diese Serie fokussieren
chart-unfocus-series = Alle Serien anzeigen

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
sp-lede = Durchsuche die Parameter, mit denen dieser Server Suchen auflöst, gefiltert nach Basis-Ressourcentyp. Gespeicherte Parameter lassen sich anlegen, bearbeiten und löschen; die Registry übernimmt Änderungen pro Tenant.
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
sp-new = Neuer Suchparameter
sp-edit = Bearbeiten
sp-delete = Löschen
sp-delete-confirm = Diesen gespeicherten Suchparameter löschen? Suchen, die ihn verwenden, finden nach der Aktualisierung der Registry keine Treffer mehr.
cmp-new = Neue Compartment-Definition
cmp-edit = Bearbeiten
cmp-delete = Löschen
cmp-delete-confirm = Diese Compartment-Definition löschen? Ihre Compartment-Routen funktionieren dann nicht mehr.
crud-delete-failed = Das Element konnte nicht gelöscht werden.

## Compartment-Ansicht & Tester (#237)

cmp-heading = Compartments
cmp-lede = Die Compartment-Definitionen, mit denen dieser Server /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"}-Anfragen routet, und ein Tester, der beantwortet: Ist dieser Typ in diesem Compartment, über welche Parameter, und welche Suche führt der Server aus?
cmp-rail-label = Compartment-Definitionen
cmp-rail-heading = Compartments
cmp-degraded = Die Compartment-Definitionen konnten gerade nicht von diesem Server geladen werden — der Selbstaufruf an /CompartmentDefinition schlug fehl (bei aktivierter Authentifizierung fehlt meist das ausgehende Service-Token oder es ist ungültig). Die Seite versucht es bei der nächsten Anfrage erneut.
cmp-rail-note = Die Definitionen sind gespeicherte Ressourcen, beim Start aus der FHIR-Spezifikation angelegt. Bearbeiten und Löschen wirken hier pro Tenant.
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
queries-invalid-fhir-escape = Diese Abfrage enthält eine ungültige FHIR-Escapesequenz. Korrigiere den maskierten Wert, bevor du ihn visuell bearbeitest.

queries-conditions = Bedingungen
queries-add-condition = Bedingung hinzufügen
queries-includes = Includes
queries-result-controls = Ergebnis-Steuerung
queries-remove = Entfernen
queries-match-is = ist
queries-or = + oder
plain-pill = In einfachen Worten
plain-find = Finde {"{type}"}-Einträge
plain-clause = {"{path}"} {"{verb}"} {"{value}"}
plain-clause-no-value = {"{path}"} {"{verb}"}
plain-and = und
plain-or = oder
plain-arrow = {" "}→
plain-has = die ein verknüpftes {"{type}"} haben, dessen {"{param}"} {"{verb}"} {"{value}"}
plain-has-no-value = die ein verknüpftes {"{type}"} haben, dessen {"{param}"} {"{verb}"}
plain-include = Zusätzlich wird der {"{param}"} jedes {"{type}"} zurückgegeben{"{target}"}
plain-revinclude = Plus jedes {"{type}"}, dessen {"{param}"} hierher zeigt
plain-iterate = (wiederholt)
plain-count = Zeigt {"{n}"} pro Seite
plain-sort = Sortiert nach {"{sort}"}
plain-verb-is = ist
plain-verb-contains = enthält
plain-verb-exact = ist genau
plain-verb-missing = ist vorhanden/fehlt
plain-verb-missing-true = fehlt
plain-verb-missing-false = ist vorhanden
plain-verb-not = ist nicht
plain-verb-text = entspricht dem Text
plain-verb-in = ist im Value Set
plain-verb-not-in = ist nicht im Value Set
plain-verb-identifier = hat den Identifier
plain-verb-of-type = hat einen Identifier vom Typ
plain-verb-ge = ist am oder nach
plain-verb-le = ist am oder vor
plain-verb-gt = ist nach
plain-verb-lt = ist vor
plain-verb-ne = ist nicht
plain-verb-eq = ist
plain-verb-sa = beginnt nach
plain-verb-eb = endet vor
plain-verb-ap = ist ungefähr
queries-related-heading = Verwandte Daten einbeziehen
queries-related-sub = Fügt verbundene Ressourcen zu den Ergebnissen hinzu.
queries-related-add-include = Eine referenzierte Ressource einbeziehen
queries-related-add-revinclude = Hierher verweisende Ressourcen einbeziehen
queries-iterate = Iterieren
queries-sort-label = Sortierung
queries-sort-default = Standard
queries-sort-recent = Neueste zuerst
queries-sort-oldest = Älteste zuerst
queries-sort-id = ID
queries-modify-heading = Modifikatoren
queries-mod-exact = ganzer Wert inkl. Groß-/Kleinschreibung & Akzente
queries-mod-contains = Treffer irgendwo im Text
queries-mod-missing = Feld ist vorhanden / fehlt
queries-mod-text = erweiterte Textbehandlung
queries-mod-not = keiner der Werte trifft zu
queries-mod-above = dieser oder ein Vorfahr
queries-mod-below = dieser oder ein Nachfahre
queries-mod-in = Mitglied des Value Sets
queries-mod-not-in = kein Mitglied des Value Sets
queries-mod-identifier = Referenz nach Identifier abgleichen
queries-mod-of-type = Identifier-Typ, -System und -Wert abgleichen
queries-chain-into = Nach einer Eigenschaft der referenzierten Ressource filtern
queries-chain-any-target = beliebig
queries-has-pill = hat eine verknüpfte
queries-has-type-placeholder = Ressourcentyp
queries-has-via = verknüpft über
queries-has-where = wobei ihr
queries-add-has = ⧉ Eine hierher verweisende Ressource filtern
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
queries-results-fetch-error = Ergebnisse von { $origin } konnten nicht geladen werden. Prüfen Sie HFS_BASE_URL und versuchen Sie es erneut.

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

## Ressourcen-Editor (#264)

editor-heading = Ressourcen-Editor
editor-lede = Bearbeiten Sie eine Ressource anhand ihres Schemas: fügen Sie jedes vom Schema erlaubte Element in beliebiger Tiefe hinzu — auch Extensions, an jedem Knoten, der sie zulässt.
editor-title = Ressource bearbeiten
editor-view-label = Bearbeitungsmodus
editor-view-form = Geführtes Formular
editor-view-json = JSON
editor-save = Änderungen speichern
editor-delete = Löschen
editor-remove = Diesen Knoten entfernen
editor-saved = Gespeichert.
editor-load-error = Diese Ressource konnte nicht geladen werden.
editor-confirm-delete = Diese Ressource löschen? Das lässt sich nicht rückgängig machen.
editor-invalid-json = Das ist kein gültiges JSON und kann daher nicht als Formular bearbeitet werden. Ihr Text bleibt unverändert.
editor-source-hint = Bearbeiten Sie den Quelltext direkt. Beim Zurückwechseln wird er geparst.

editor-add = Element hinzufügen
editor-must-support-badge = MS
editor-binding-hint = An ein Value Set gebunden — Codes stammen daraus; Stärke angezeigt
editor-legend-live = Beim Tippen geprüft: Struktur, Kardinalität, erforderliche Bindings
editor-legend-save = Beim Speichern geprüft: Constraints und Terminologie
editor-deferred-badge = beim Speichern
editor-deferred-hint = Codes werden beim Speichern gegen das Value Set geprüft (und live im Picker, wenn ein Terminologieserver konfiguriert ist)
editor-must-support-hint = Must-support: Konsumenten dieses Profils müssen dieses Element verarbeiten können
editor-add-filter = Elemente filtern
editor-add-another = weiteres hinzufügen
editor-pick-type = Typ wählen…
editor-extension-url = Extension-URL
editor-add-extension = Extension hinzufügen

editor-valid = Keine Probleme.
editor-issues = { $count ->
    [one] { $count } Problem
   *[other] { $count } Probleme
}

editor-modifier-badge = Modifier
editor-modifier-warning = Eine Modifier-Extension ändert die Bedeutung der Ressource. Ein System, das sie nicht kennt, muss die Verarbeitung verweigern.
editor-unknown-badge = nicht im Schema
editor-unknown-hint = Das Schema beschreibt dieses Element nicht. Es wird angezeigt, damit es nicht stillschweigend verloren geht, und beim Speichern erhalten.

editor-primitive-extension-badge = + Extension
editor-primitive-extension-hint = Dieser Wert trägt eigene Extensions (ein `_`-Geschwister im JSON). Sie bleiben beim Speichern erhalten.

editor-collapse-all = Alle einklappen
editor-expand-all = Alle ausklappen
json-view-toggle-fold = JSON-Abschnitt umschalten
editor-edit-raw = Rohtext bearbeiten
editor-versions = Versionen
editor-versions-none = Keine früheren Versionen.
## Verlauf & Versionen (#236)

## Ressourcen-Arbeitsbereich (#282)

resources-heading = Ressourcen
resources-lede = FHIR-Ressourcen durchsuchen, suchen, erstellen und bearbeiten. In natürlicher Sprache suchen oder die Abfrage selbst bauen, dann ein Ergebnis zum Bearbeiten öffnen.
resources-create = Ressource erstellen
resources-create-typed = { $type } erstellen
resources-create-invalid-type = Dieser Ressourcentyp ist in der ausgewählten FHIR-Version nicht verfügbar. Korrigieren Sie die Abfrage oder wählen Sie einen Typ aus der Liste.
resources-create-not-advertised = Dieser Server erlaubt das Erstellen dieses Ressourcentyps nicht. Sie können ihn weiterhin durchsuchen.
resources-create-schema-unavailable = Für diesen Ressourcentyp gibt es in der ausgewählten FHIR-Version kein Editorschema. Die UI kann ihn deshalb nicht sicher erstellen.
resources-create-metadata-unavailable = Die Serverfähigkeiten sind nicht verfügbar. Das Erstellen bleibt deaktiviert, bis die UI sie prüfen kann.
resources-save-blocked = Beheben Sie die Validierungsprobleme vor dem Speichern.
resources-save-invalid = Das JSON ist ungültig — beheben Sie es vor dem Speichern.
resources-edit-title = Ressource bearbeiten
resources-tab-edit = Bearbeiten
resources-tab-history = Verlauf
resources-types-heading = Ressourcentypen
rail-all-types-heading = Alle Typen

queries-saved-group = Gespeichert

nav-collapse = Menü einklappen

batch-heading = Batch / Transaction
batch-lede = Lade ein FHIR-Bundle hoch, prüfe die auszuführenden Aktionen, führe es gegen diesen Server aus und lies das Ergebnis jedes Eintrags.
batch-upload = Hochladen
batch-drop-hint = Bundle-JSON-Datei hier ablegen
batch-drop-browse = oder klicken zum Durchsuchen
batch-invalid-json = Diese Datei ist kein gültiges JSON.
batch-not-a-bundle = Dieses JSON ist kein FHIR-Bundle.
batch-bad-type = Hier lassen sich nur Bundles vom Typ batch oder transaction ausführen.
batch-request = Anfrage
batch-entries = Einträge
batch-semantics-batch = Batch: Einträge laufen unabhängig — ein fehlgeschlagener Eintrag stoppt die anderen nicht und macht sie nicht rückgängig.
batch-semantics-transaction = Transaction: alles oder nichts — schlägt ein Eintrag fehl, rollt der Server das gesamte Bundle zurück.
batch-tab-actions = Aktionen
batch-tab-json = Bundle-JSON
batch-no-body = (kein Body — dieser Eintrag adressiert nur eine Ressource)
batch-cancel = Abbrechen
batch-execute = Ausführen
batch-plan-heading = Ausführungsplan
batch-done = Fertig
batch-response-heading = Ergebnisse pro Aktion
batch-sum-created = erstellt
batch-sum-updated = aktualisiert
batch-sum-other = gelesen/sonstige
batch-sum-failed = fehlgeschlagen
batch-request-failed = Die Anfrage ist fehlgeschlagen.
batch-reading = Bundle wird gelesen…
batch-executing = Wird ausgeführt…
batch-read-failed = Die Datei konnte nicht gelesen werden.

## Bulk Import workspace (#527)

bulk-import-title = Massenimport
bulk-import-lede = Vorkoordinierte FHIR-Datensätze mit der Bulk-Data-Operation $bulk-submit an einen Data Recipient senden.
bulk-import-detail-lede = Manifeste, Status und Ausführungsprotokoll dieser Übermittlung.
bulk-import-new = Neue Submission
bulk-import-create-title = Bulk Submission anlegen
bulk-import-field-name = Name der Submission
bulk-import-auth = Authentifizierung
bulk-import-auth-hint = Wie gegenüber dem Empfängerserver authentifiziert wird.
bulk-import-auth-none = Keine
bulk-import-auth-none-hint = Es wird kein Authorization-Header gesendet.
bulk-import-auth-backend = Backend-Services-Authentifizierung
bulk-import-auth-backend-hint = Holt ein Zugriffstoken und sendet es als Bearer im Authorization-Header.
bulk-import-field-client-id = Client-ID
bulk-import-field-client-id-hint = Registrieren Sie diesen Datenanbieter beim Empfänger und erhalten Sie eine Client-ID.
bulk-import-field-token-url = Token-URL
bulk-import-field-token-url-hint = Token-Endpunkt-URL des Autorisierungsservers.
bulk-import-jwks-hint = Registrieren Sie den öffentlichen Schlüssel dieses Servers beim Empfänger über die JWKS-URL:
bulk-import-test-auth = Authentifizierung testen
bulk-import-test-auth-ok = Authentifizierung erfolgreich.
bulk-import-create-submit = Erstellen & Senden
bulk-import-advanced = Erweiterte Optionen
bulk-import-unavailable = Das Storage-Backend hostet keinen Settings-Store; Submissions können nicht gespeichert werden.
bulk-import-submissions = Submissions
bulk-import-records = Einträge
bulk-import-col-name = Name
bulk-import-col-status = Status
bulk-import-col-created = Erstellt
bulk-import-col-destination = Ziel
bulk-import-empty = Noch keine Submissions. Legen Sie eine an, um zu beginnen.
bulk-import-all = Alle Submissions
bulk-import-status-not-started = Nicht gestartet
bulk-import-status-in-progress = In Bearbeitung
bulk-import-status-stopped = Angehalten
bulk-import-status-completed = Abgeschlossen
bulk-import-status-failed = Fehlgeschlagen
bulk-import-detail-recipient = Datenempfänger
bulk-import-detail-id = Submission-ID
bulk-import-detail-submitter = Einreicher
bulk-import-detail-created = Erstellt
bulk-import-detail-status = Status
bulk-import-detail-auth = Authentifizierung
bulk-import-abort = Abbrechen
bulk-import-delete = Löschen
bulk-import-edit = Bearbeiten
bulk-import-edit-title = Submission bearbeiten
bulk-import-edit-submit = Änderungen speichern
bulk-import-field-manifest-url = Manifest-URL
bulk-import-field-manifest-url-hint = URL eines Bulk-Export-Manifests mit einem vorkoordinierten FHIR-Datensatz.
bulk-import-field-output-format = Format
bulk-import-field-output-format-hint = Das Format der Bulk-Data-Dateien im Manifest.
bulk-import-field-headers = Header für Dateiabrufe
bulk-import-field-headers-hint = HTTP-Header, die der Empfänger beim Abruf einer Datendatei verwenden soll, je Zeile "Name: Wert".
bulk-import-log = Submission-Protokoll
bulk-import-log-empty = Noch nichts übermittelt.
bulk-import-field-submitter-system = Einreicher-System
bulk-import-field-submitter-value = Einreicher-Wert
bulk-import-field-submitter-hint = Muss einem beim Empfänger registrierten Identifier entsprechen (außerhalb des Protokolls abgestimmt). Leer lassen für die generierten Standardwerte.
bulk-import-processing = Verarbeitung
bulk-import-processing-waiting = Warten auf den ersten Statusbericht des Empfängers …
bulk-import-result = Ergebnis
bulk-import-result-finished = Verarbeitung abgeschlossen um
bulk-import-result-outputs = Ausgabedateien
bulk-import-result-errors = Fehlerdateien
ui-cancel = Abbrechen
ui-close = Schließen
editor-orphans-title = Diese Probleme haben noch kein Feld — fügen Sie die Elemente hinzu, um sie zu beheben
editor-hint-date = FHIR date: YYYY, YYYY-MM oder YYYY-MM-DD
editor-hint-datetime = FHIR dateTime: YYYY, YYYY-MM, YYYY-MM-DD oder ein vollständiger Zeitstempel mit Zeitzone (2024-05-17T14:30:00+02:00)
editor-hint-time = FHIR time: HH:MM:SS
editor-hint-instant = FHIR instant: vollständiger Zeitstempel mit Zeitzone, z. B. 2024-05-17T14:30:00.000Z

## Abonnement-Seite (#580)

subs-title = Abonnements
subs-lede = Schreibgeschützte Sicht auf die Abonnement-Engine: jedes registrierte Abonnement, sein Kanal, Live-Status und Zustellzähler.
subs-unavailable = Die Abonnement-Engine ist auf diesem Server nicht aktiviert.
subs-unavailable-how = Zum Aktivieren HFS starten mit:
subs-empty = Für diesen Mandanten sind keine Abonnements registriert.
subs-card-failing = Fehlgeschlagen
subs-card-failing-sub = Braucht Aufmerksamkeit
subs-card-idle = Inaktiv
subs-card-idle-sub = Keine Clients
subs-card-active = Aktiv
subs-card-active-sub = wird zugestellt
subs-card-delivered = Zugestellt in 24 h
subs-card-delivered-sub = { $rate }% beim ersten Versuch
subs-card-delivered-none = keine Zustellungen im Fenster
subs-table-heading = Abonnements
subs-sort = Sortieren
subs-sort-status = Status
subs-sort-sent = Meistgesendet
subs-sort-fails = Fehlerserie
subs-col-subscription = Abonnement
subs-col-channel = Kanal
subs-col-status = Status
subs-col-last24 = Letzte 24 h
subs-col-sent = Gesendet
subs-col-fails = Fehlerserie
subs-state-active = Aktiv
subs-state-error = Fehler
subs-state-idle = 0 Clients
subs-state-requested = Angefragt
subs-state-off = Aus

## Bulk Export workspace (#537)

bulk-export-title = Massenexport
bulk-export-lede = Daten mit der FHIR-Bulk-Data-Operation $export als NDJSON-Dateien aus diesem Server exportieren.
bulk-export-active-title = Aktive Exporte
bulk-export-active-link = Aktive Exporte
bulk-export-new = Neuer Export
bulk-export-unavailable = Das Storage-Backend hostet keinen Settings-Store; Exportaufträge können nicht verfolgt werden.
bulk-export-scope = Was möchten Sie exportieren?
bulk-export-scope-system = Alles
bulk-export-scope-system-hint = Der gesamte Server — jeder unten ausgewählte Ressourcentyp.
bulk-export-scope-patient = Patienten
bulk-export-scope-patient-hint = Jeder Patient und die zugehörigen Datensätze. Nichts Patientenfremdes.
bulk-export-scope-group = Gruppe
bulk-export-scope-group-hint = Nur die Mitglieder einer bereits definierten Kohorte.
bulk-export-field-group-id = Gruppen-ID
bulk-export-field-group-id-hint = Erforderlich für den Gruppen-Umfang: die ID der zu exportierenden FHIR-Group.
bulk-export-field-name = Name
bulk-export-field-name-placeholder = Diabetes-Register 2024
bulk-export-types = Ressourcentypen
bulk-export-types-hint = Nichts ankreuzen, um alle Typen zu exportieren.
bulk-export-narrow = Eingrenzen
bulk-export-field-elements = FHIR-Elemente
bulk-export-field-type-filter = Typfilter
bulk-export-field-since = Seit
bulk-export-since-all = Gesamter Zeitraum
bulk-export-since-day = Letzter Tag
bulk-export-since-week = Letzte 7 Tage
bulk-export-since-month = Letzte 4 Wochen
bulk-export-since-custom = Benutzerdefiniert
bulk-export-field-since-custom = Benutzerdefinierter Zeitpunkt
bulk-export-field-since-custom-hint = Gilt, wenn Seit auf Benutzerdefiniert steht. RFC 3339, z. B. 2026-08-01T00:00:00Z.
bulk-export-start = Export starten
bulk-export-running = laufend
bulk-export-clear = Leeren
bulk-export-files-word = Dateien
bulk-export-exports-word = Exporte
bulk-export-none = Noch keine Exporte. Starten Sie einen auf der Massenexport-Seite.
bulk-export-status-in-progress = Läuft
bulk-export-status-complete = Abgeschlossen
bulk-export-status-failed = Fehlgeschlagen
bulk-export-status-cancelled = Abgebrochen
bulk-export-progress = Fortschritt
bulk-export-progress-waiting = Warten auf den ersten Statusbericht …
bulk-export-files = Dateien
bulk-export-finished-in = fertig in
bulk-export-error = Fehler
bulk-export-cancel = Abbrechen
bulk-export-retry = Erneut versuchen

# CapabilityStatement-Seite (#653)
cap-title = Capability Statement
cap-lede = Was dieser Server aktuell leistet, für den gewählten Tenant und die FHIR-Version — live aus /metadata zusammengesetzt.
cap-summary-heading = Server-Übersicht
cap-summary-description = Beschreibung
cap-summary-url = Basis-URL
cap-summary-fhir-version = FHIR-Version
cap-summary-status = Status
cap-summary-kind = Art
cap-summary-date = Datum
cap-summary-formats = Formate
cap-interactions-heading = System-Interaktionen
cap-transaction-note = transaction wird angeboten, weil das aktive Backend atomare Transaktionen unterstützt; batch ist immer verfügbar.
cap-role-matrix = Backend-Rollenmatrix anzeigen.
cap-operations-heading = Operationen
cap-col-operation = Operation
cap-col-definition = Definition
cap-resources-heading = Fähigkeiten pro Ressource
cap-filter-placeholder = Typen filtern…
cap-col-type = Typ
cap-col-interactions = Interaktionen
cap-col-search-params = Suchparameter
cap-col-includes = Includes
cap-col-revincludes = Revincludes
cap-resources-empty = Kein Ressourcentyp entspricht dem Filter.
cap-raw-toggle = Rohes CapabilityStatement (JSON)
cap-unavailable = Das CapabilityStatement konnte nicht vom Server geladen werden — der Selbstaufruf benötigt bei aktivierter Authentifizierung eventuell ein Ausgangs-Token.

## Stubs der SQL-on-FHIR-Sektion (#649)


sql-vd-title = View-Definitionen
sql-vd-lede = Erstelle und verwalte die ViewDefinitions, mit denen SQL on FHIR Ressourcen abflacht.

sql-queries-title = SQL-Abfragen
sql-queries-lede = Führe SQL-on-FHIR-Abfragen gegen diesen Server aus.

sql-views-title = SQL-Views
sql-views-lede = Wiederverwendbare SQL-Views auf Basis von ViewDefinitions.

sql-export-title = SQL-Export
sql-export-lede = Langlaufende SQL-on-FHIR-Exportaufträge.

sql-files-title = Dateien
sql-files-lede = Manifeste und Ausgabedateien der SQL-Exporte.

## View-Definitionen-Arbeitsbereich (#649)

vd-new = Neu erstellen
vd-new-title = Neue View-Definition
vd-rail-label = View-Definitionen
vd-rail-heading = View-Definitionen
vd-rail-all-heading = Alle View-Definitionen
vd-filter = Views filtern
vd-none = Noch keine View-Definitionen.
vd-empty-lede = Lege mit „Neu erstellen" die erste ViewDefinition an.
vd-degraded = Die Liste der View-Definitionen konnte nicht geladen werden.
vd-saved = Gespeichert.
vd-run = Ausführen
vd-run-failed = Die View konnte nicht ausgeführt werden.
vd-save = Speichern
vd-duplicate = Duplizieren
vd-delete = Löschen
vd-delete-confirm = View-Definition „{ $name }" löschen? Das kann nicht rückgängig gemacht werden.
vd-delete-failed = Die View-Definition konnte nicht gelöscht werden.
vd-json-heading = Definition (JSON)
vd-results-heading = Ergebnisse
vd-results-empty = Die View hat keine Zeilen erzeugt.
vd-pagination-label = View-Definitionsseiten
vd-page-prev = Zurück
vd-page-next = Weiter

## SQL-Abfragen- / SQL-Views-Arbeitsbereiche (#649)

sql-queries-new-title = Neue SQL-Abfrage
sql-views-new-title = Neue SQL-View
sql-queries-rail-all-heading = Alle SQL-Abfragen
sql-views-rail-all-heading = Alle SQL-Views
lib-filter = Bibliotheken filtern
lib-none = Noch keine Bibliotheken.
lib-empty-lede = Lege mit „Neu erstellen" die erste Bibliothek an.
lib-degraded = Die Bibliotheksliste konnte nicht geladen werden.
lib-sql-heading = SQL
lib-delete-confirm = „{ $name }" löschen? Das kann nicht rückgängig gemacht werden.
lib-delete-failed = Die Bibliothek konnte nicht gelöscht werden.

## SQL-Export- und Dateien-Seiten (#649)

export-start-failed = Der Export konnte nicht gestartet werden.
export-started = Export gestartet.
export-cancelled = Abbruch angefordert.
export-job-heading = Exportauftrag
export-job-id = Auftrags-ID
export-job-state = Status
export-state-running = Läuft
export-state-done = Abgeschlossen
export-state-unknown = Unbekannter Auftrag — möglicherweise abgebrochen oder bereinigt.
export-refresh = Aktualisieren
export-cancel = Auftrag abbrechen
export-view-files = Dateien anzeigen
export-new-heading = Neuer Export
export-no-subjects = Noch nichts zu exportieren — lege zuerst eine ViewDefinition an.
export-format = Ausgabeformat
export-start = Export starten
files-job-heading = Exportauftrag
files-load = Manifest laden
files-error = Das Manifest konnte nicht geladen werden.
files-outputs-heading = Ausgaben
files-col-output = Ausgabe
files-col-downloads = Downloads
files-shard = Datei { $n }
files-empty = Der Auftrag hat keine Ausgabedateien erzeugt.

## Administrative HTS-UI (crates/hts-ui) — Phase-1-Stubs
##
## Schlüssel für die HTS-UI folgen der Konvention
## hts-<seite>-<rolle>-<control>. Diese Stubs decken Base-Layout,
## Seitennavigation und den Dashboard-Platzhalter der Phase-1-Blocker-Slice ab.
## Sie müssen paritätisch zu en/es/main.ftl bleiben.

-hts-app-name = Helios Terminologieserver
hts-app-title = { -hts-app-name }

hts-nav-section-work = Terminologie
hts-nav-section-tools = Werkzeuge
hts-nav-section-server = Server
hts-nav-home = Startseite
hts-nav-code-systems = Codesysteme
hts-nav-value-sets = Wertemengen
hts-nav-concept-maps = Konzeptzuordnungen
hts-nav-operations = Operationen
hts-nav-import = Import

hts-fhir-version-heading = FHIR-Version
hts-fhir-version = FHIR { $version }

hts-home-title = Startseite
hts-home-subtitle = Zustand des Terminologieservers, Katalogbestand und Schnellaktionen.

## Dashboard-Zeilen (visuell verborgen, nur für Screenreader).

hts-home-row-status = Serverstatus

## Dashboard-Kacheln.

hts-home-tile-status = Status
hts-home-tile-uptime = Laufzeit
hts-home-tile-loaded-systems = Geladene Codesysteme
hts-home-tile-loaded-systems-hint = Aus TerminologyCapabilities.codeSystem[]
hts-home-tile-requests = Anfragen
hts-home-tile-metrics-hint = Seit Serverstart

## Anfragerate-Diagramm der Startseite (Design-Dokument §7.1). Zeigt eine aus
## den kumulativen `/metrics`-Zählern gebildete Rate, die nur erfasst wird,
## solange diese Seite geöffnet ist — daher braucht jeder „nichts zu
## zeichnen“-Zustand einen eigenen Text.

hts-home-chart-title = Anfragen pro Minute
hts-home-chart-window = Zeitfenster des Diagramms
hts-home-chart-series = Statusklasse
hts-home-chart-window-15m = 15 Min.
hts-home-chart-window-1h = 1 Std.
hts-home-chart-window-6h = 6 Std.
hts-home-chart-series-all = Alle
hts-home-chart-series-2xx = 2xx
hts-home-chart-series-4xx = 4xx
hts-home-chart-series-5xx = 5xx
hts-home-chart-empty-unreachable = /metrics ist nicht erreichbar – es kommen keine neuen Messwerte an.
hts-home-chart-empty-none = Noch keine Messwerte erfasst.
hts-home-chart-empty-first = Erstes Intervall wird erfasst — eine Rate benötigt zwei Messwerte.
hts-home-chart-empty-window = Keine Messwerte in diesem Zeitfenster. Die Erfassung läuft nur, solange diese Seite geöffnet ist.
hts-home-chart-axis-now = jetzt
hts-home-chart-axis-minutes = -{ $n } Min.
hts-home-chart-axis-hours = -{ $n } Std.

## `status`-Werte aus /health, per Schlüssel übersetzbar.

hts-home-status-ok = OK

## Degradiert-Banner (Design-Dokument §7-Kontrakt).

hts-degraded-title = Das Terminologie-Backend ist nicht vollständig verfügbar
hts-degraded-body = Einige Kacheln werden ausgeblendet, bis HTS wieder erreichbar ist. Interaktive Bedienelemente sind auf betroffenen Seiten deaktiviert.
hts-degraded-reason-client-build = Der ausgehende HTTP-Client konnte nicht erstellt werden.
hts-degraded-reason-upstream-down = Der Terminologieserver ist nicht erreichbar.
hts-degraded-reason-upstream-timeout = Der Terminologieserver hat nicht rechtzeitig geantwortet.
hts-degraded-reason-upstream-error = Der Terminologieserver hat einen Fehlerstatus zurückgegeben.
hts-degraded-reason-upstream-shape = Der Terminologieserver hat eine Antwort in unerwarteter Form zurückgegeben.
hts-degraded-reason-bootstrapping = Der Terminologieserver lädt noch seine Ausgangsdaten.
hts-degraded-reason-unknown = Der Terminologieserver ist vorübergehend nicht verfügbar.

## Dialekt-Chip (Topbar, sitzungsweiter displayLanguage / Accept-Language — §7.1).


## OperationOutcome-Partial (gemeinsam — §7 / §11).

hts-outcome-severity = Schweregrad: { $severity }
hts-outcome-request-id = Anfrage-ID: { $id }
hts-outcome-code-not-found = Die angeforderte Ressource wurde nicht gefunden.
hts-outcome-code-invalid = Die Anfrage wurde als ungültig zurückgewiesen.
hts-outcome-code-too-costly = Die angeforderte Operation wurde als zu teuer zurückgewiesen.
hts-outcome-code-unknown = Der Server hat ein Problem gemeldet, das die UI nicht kennt.
hts-degraded-since = Seit { $timestamp }

## HTS Slice B — CodeSystem-Browser + Detailansicht mit eingebettetem Workbench
## (Design-Dokument §7.2 + §7.3). Jeder Schlüssel hat ein Pendant in en/es/main.ftl.

## CodeSystem-Statuspillen (Browser-Zeilen und Detail-Kopfzeile).

hts-cs-status-draft = Entwurf
hts-cs-status-active = aktiv
hts-cs-status-retired = zurückgezogen
hts-cs-status-unknown = unbekannt

## CodeSystem-Browserseite.

hts-cs-browser-title = Codesysteme
hts-cs-browser-subtitle = Durchsuche den CodeSystem-Katalog des Terminologieservers und öffne eine Zeile, um Metadaten und Workbench einzusehen.
hts-cs-browser-filter-legend = CodeSysteme filtern
hts-cs-browser-filter-url = Kanonische URL
hts-cs-browser-filter-version = Version
hts-cs-browser-filter-name = Name
hts-cs-browser-filter-title = Titel
hts-cs-browser-filter-status = Status
hts-cs-browser-filter-search = Suchen
hts-cs-browser-filter-reset = Zurücksetzen
hts-cs-browser-empty = Keine CodeSysteme entsprechen diesen Filtern.
hts-cs-browser-load-more = Mehr laden
hts-cs-browser-showing-count = Es werden { $count ->
    [one] { $count } CodeSystem angezeigt
   *[other] { $count } CodeSysteme angezeigt
}
hts-cs-browser-table-caption = CodeSysteme, die zu den aktiven Filtern passen.
hts-cs-browser-column-url = URL
hts-cs-browser-column-version = Version
hts-cs-browser-column-title = Titel
hts-cs-browser-column-status = Status
hts-cs-browser-column-name = Name

## Phase 5 — HTS-Suchformular: gemeinsame Zeichenketten (CS / VS / CM).

hts-search-rail-label = Suchfilter
hts-search-rail-heading = Filter
hts-facet-status-any = Jeder Status

## CodeSystem-Detailseite.

hts-cs-detail-title = { $name } · CodeSystem
hts-cs-detail-title-fallback = CodeSystem
hts-cs-detail-eyebrow = CodeSystem
hts-cs-detail-section-identity = Identität
hts-cs-detail-section-content = Inhalt
hts-cs-detail-content-mode = Inhaltsmodus
hts-cs-detail-count = Anzahl Konzepte
hts-cs-detail-publisher = Herausgeber
hts-cs-detail-jurisdiction = Zuständigkeit
hts-cs-detail-supersedes = Ersetzt
hts-cs-detail-superseded-by = Ersetzt durch
hts-cs-detail-tabs-label = CodeSystem-Workbench-Abschnitte
hts-cs-detail-tab-lookup = Nachschlagen
hts-cs-detail-tab-validate = Validieren
hts-cs-detail-tab-subsumes = Subsumption
hts-cs-detail-result-empty = Führe die Operation aus, um das Ergebnis hier zu sehen.

## $lookup-Formular + Ergebnisbeschriftungen.

hts-cs-lookup-heading = Konzept nachschlagen
hts-cs-lookup-code = Code
hts-cs-lookup-version = Version
hts-cs-lookup-display-language = Anzeigesprache
hts-cs-lookup-display-language-placeholder = z. B. de-DE
hts-cs-lookup-properties-legend = Eigenschaften
hts-cs-lookup-designations = Bezeichnungen
hts-cs-lookup-properties = Eigenschaften
hts-cs-lookup-no-match = HTS hat kein passendes Konzept zurückgegeben.

## $validate-code-Formular + Ergebnisbeschriftungen.

hts-cs-validate-heading = Code validieren
hts-cs-validate-mode-legend = Eingabemodus
hts-cs-validate-mode-code = Einzelcode
hts-cs-validate-mode-coding = Coding
hts-cs-validate-code = Code
hts-cs-validate-display = Anzeige
hts-cs-validate-coding-legend = Coding
hts-cs-validate-coding-system = System
hts-cs-validate-coding-code = Code
hts-cs-validate-coding-display = Anzeige
hts-cs-validate-badge-true = gültig
hts-cs-validate-badge-false = ungültig
hts-cs-validate-message = Meldung

## $subsumes-Formular + Ergebnisbeschriftungen.

hts-cs-subsumes-heading = Subsumption prüfen
hts-cs-subsumes-scoped-system = System (festgelegt)
hts-cs-subsumes-code-a = Code A
hts-cs-subsumes-code-b = Code B
hts-cs-subsumes-outcome-equivalent = Die Codes sind äquivalent.
hts-cs-subsumes-outcome-subsumes = Code A subsumiert Code B.
hts-cs-subsumes-outcome-subsumed-by = Code A wird von Code B subsumiert.
hts-cs-subsumes-outcome-not-subsumed = Keiner der Codes subsumiert den anderen.

## Geteilte Workbench-Chrome (auch für Slice C/D/E).

hts-workbench-run = Ausführen
hts-workbench-raw-response = Rohanfrage und -antwort
hts-workbench-copy-url = Anfrage-URL

## Zusätzlicher Degradiert-Grund für 404 beim CodeSystem-Read (§7.3).

hts-degraded-reason-upstream-not-found = Der Terminologieserver hat diese Ressource nicht gefunden.

## HTS Slice C — ValueSet-Browser + Detailseite mit $expand-Werkbank
## (design doc §7.4 + §7.4.1). Jeder Schlüssel hat ein Pendant in en/es/main.ftl.

## Statusabzeichen für ValueSet.

hts-vs-status-draft = Entwurf
hts-vs-status-active = aktiv
hts-vs-status-retired = zurückgezogen
hts-vs-status-unknown = unbekannt

## VS-Browser-Seite.

hts-vs-browser-title = ValueSets
hts-vs-browser-subtitle = Durchsuche den ValueSet-Katalog des Terminologieservers und öffne eine Zeile, um Metadaten oder eine Expansion einzusehen.
hts-vs-browser-filter-legend = ValueSets filtern
hts-vs-browser-filter-url = Kanonische URL
hts-vs-browser-filter-version = Version
hts-vs-browser-filter-name = Name
hts-vs-browser-filter-title = Titel
hts-vs-browser-filter-status = Status
hts-vs-browser-filter-search = Suchen
hts-vs-browser-filter-reset = Zurücksetzen
hts-vs-browser-empty = Keine ValueSets für diese Filter.
hts-vs-browser-load-more = Mehr laden
hts-vs-browser-showing-count = Zeige { $count ->
    [one] { $count } ValueSet
   *[other] { $count } ValueSets
}
hts-vs-browser-table-caption = ValueSets, die den aktiven Filtern entsprechen.
hts-vs-browser-column-url = URL
hts-vs-browser-column-version = Version
hts-vs-browser-column-title = Titel
hts-vs-browser-column-status = Status
hts-vs-browser-column-name = Name

## VS-Detailseite.

hts-vs-detail-title = { $name } · ValueSet
hts-vs-detail-title-fallback = ValueSet
hts-vs-detail-eyebrow = ValueSet
hts-vs-detail-section-identity = Identität
hts-vs-detail-section-governance = Verwaltung
hts-vs-detail-publisher = Herausgeber
hts-vs-detail-jurisdiction = Zuständigkeit
hts-vs-detail-immutable = Unveränderlich
hts-vs-detail-immutable-yes = ja
hts-vs-detail-immutable-no = nein
hts-vs-detail-purpose = Zweck
hts-vs-detail-copyright = Urheberrecht
hts-vs-detail-tabs-label = ValueSet-Werkbank-Abschnitte
hts-vs-detail-tab-expand = Expandieren
hts-vs-detail-result-empty = Führe die Operation aus, um das Ergebnis hier zu sehen.

## $expand — Formular und Ergebnisse.

hts-vs-expand-heading = Diesen ValueSet expandieren
hts-vs-expand-scoped-valueset = ValueSet (fixiert)
hts-vs-expand-filter = Filter
hts-vs-expand-filter-placeholder = Code oder Anzeigetext
hts-vs-expand-count = count
hts-vs-expand-offset = offset
hts-vs-expand-display-language = Anzeigesprache
hts-vs-expand-display-language-placeholder = z. B. de-DE
hts-vs-expand-flags-legend = Optionen
hts-vs-expand-active-only = Nur aktive Konzepte
hts-vs-expand-include-designations = Designationen einschließen
hts-vs-expand-mode-legend = Ergebnisformat
hts-vs-expand-mode-flat = Flach
hts-vs-expand-mode-tree = Baum
hts-vs-expand-use-supplement-legend = Ergänzungen anwenden
hts-vs-expand-use-supplement-placeholder = Kanonische URL
hts-vs-expand-advanced-summary = Erweitert
hts-vs-expand-date = Datum
hts-vs-expand-date-placeholder = ISO 8601 (z. B. 2025-06-01)
hts-vs-expand-property-legend = Eigenschaften
hts-vs-expand-property-placeholder = Eigenschaftscode
hts-vs-expand-tx-resource-legend = tx-resource
hts-vs-expand-tx-resource-placeholder = Kanonische URL oder Referenz
hts-vs-expand-system-version-legend = system-version
hts-vs-expand-system-version-placeholder = System|Version
hts-vs-expand-check-system-version-legend = check-system-version
hts-vs-expand-force-system-version-legend = force-system-version
hts-vs-expand-default-valueset-version = default-valueset-version
hts-vs-expand-threshold = Too-costly-Schwelle
hts-vs-expand-ceiling-tooltip = UI-Obergrenze: { $ceiling } (höhere Werte werden verworfen)
hts-vs-expand-ceiling-note = Obergrenze: { $ceiling }
hts-vs-expand-ceiling-warning-title = Schwelle über der UI-Obergrenze
hts-vs-expand-ceiling-warning-body = Schwelle { $requested } liegt über der UI-Obergrenze — der Header wurde nicht angehängt.
hts-vs-expand-ceiling-value = Obergrenze: { $ceiling }
hts-vs-expand-too-costly-title = Expansion als zu teuer abgelehnt
hts-vs-expand-too-costly-body = HTS hat die Expansion oberhalb der aktuellen Schwelle abgelehnt. Höher setzen und erneut versuchen, oder den Filter enger fassen.
hts-vs-expand-raise-threshold = Schwelle anheben auf
hts-vs-expand-raise-submit = Erneut versuchen
hts-vs-expand-tree-label = zeige den ganzen Baum { $count ->
    [one] { $count } Blatt
   *[other] { $count } Blätter
}
hts-vs-expand-total-label = insgesamt { $total }
hts-vs-expand-total-unknown = insgesamt (unbekannt)
hts-vs-expand-offset-label = offset { $offset }
hts-vs-expand-filter-no-match = Kein Element entspricht dem Filter "{ $filter }".
hts-vs-expand-no-members = Diese Expansion enthält keine Elemente.
hts-vs-expand-column-code = Code
hts-vs-expand-column-display = Anzeige
hts-vs-expand-column-system = System
hts-vs-expand-load-more = Mehr laden
hts-vs-expand-echoed-parameters = Echo-Parameter

## HTS Slice D — ConceptMap-Browser und Detail mit eingebettetem
## $translate-Workbench (Designdokument §7.5). Jeder Schlüssel hat ein
## Pendant in en/es/main.ftl.

## ConceptMap-Status-Pillen.

hts-cm-status-draft = Entwurf
hts-cm-status-active = aktiv
hts-cm-status-retired = ausgemustert
hts-cm-status-unknown = unbekannt

## CM-Browser-Seite.

hts-cm-browser-title = ConceptMaps
hts-cm-browser-subtitle = Durchsuche den Katalog der ConceptMaps auf dem Terminologieserver und öffne eine Zeile, um Metadaten anzuzeigen oder eine Übersetzung auszuführen.
hts-cm-browser-filter-legend = ConceptMaps filtern
hts-cm-browser-filter-url = Kanonische URL
hts-cm-browser-filter-name = Name
hts-cm-browser-filter-title = Titel
hts-cm-browser-filter-status = Status
hts-cm-browser-filter-hint = Quell- und Ziel-Canonicals stehen nicht als Filter zur Verfügung: HTS akzeptiert bei der ConceptMap-Suche nur url, version, name, title und status und ignoriert alles andere. Filtern Sie nach URL oder Name und lesen Sie dann die Spalte Zuordnung.
hts-cm-browser-filter-search = Suchen
hts-cm-browser-filter-reset = Zurücksetzen
hts-cm-browser-empty = Keine ConceptMaps entsprechen diesen Filtern.
hts-cm-browser-load-more = Mehr laden
hts-cm-browser-showing-count = { $count ->
    [one] { $count } ConceptMap wird angezeigt
   *[other] { $count } ConceptMaps werden angezeigt
}
hts-cm-browser-table-caption = ConceptMaps, die den aktiven Filtern entsprechen.
hts-cm-browser-column-url = URL
hts-cm-browser-column-title = Titel
hts-cm-browser-column-status = Status
hts-cm-browser-column-name = Name
hts-cm-browser-column-source = Quellsystem
hts-cm-browser-column-target = Zielsystem
hts-cm-browser-column-mapping = Zuordnung
hts-cm-browser-mapping-source-prefix = Q:
hts-cm-browser-mapping-target-prefix = Z:

## CM-Detailseite.

hts-cm-detail-title = { $name } · ConceptMap
hts-cm-detail-title-fallback = ConceptMap
hts-cm-detail-eyebrow = ConceptMap
hts-cm-detail-section-identity = Identität
hts-cm-detail-section-mapping = Mapping
hts-cm-detail-publisher = Herausgeber
hts-cm-detail-jurisdiction = Zuständigkeit
hts-cm-detail-purpose = Zweck
hts-cm-detail-source-uri = Quelle
hts-cm-detail-target-uri = Ziel
hts-cm-detail-group-count = Gruppen
hts-cm-detail-tabs-label = Workbench-Bereiche der ConceptMap
hts-cm-detail-tab-translate = Übersetzen
hts-cm-detail-result-empty = Führe die Operation aus, um das Ergebnis hier zu sehen.

## $translate-Formular und -Ergebnisse.

hts-cm-translate-heading = Einen Code übersetzen
hts-cm-translate-scoped-map = ConceptMap (fest)
hts-cm-translate-direction-legend = Richtung
hts-cm-translate-direction-forward = Vorwärts
hts-cm-translate-direction-reverse = Rückwärts
hts-cm-translate-source-legend = Quellcodierung
hts-cm-translate-source-system = System
hts-cm-translate-source-system-placeholder = kanonische URL
hts-cm-translate-source-code = Code
hts-cm-translate-source-display = Anzeige
hts-cm-translate-source-display-placeholder = optional
hts-cm-translate-reverse-legend = Rückwärts-Quelle
hts-cm-translate-target-code = Zielcode
hts-cm-translate-target-code-hint = Im Rückwärtsmodus erforderlich.
hts-cm-translate-target-legend = Ziel-Einschränkungen
hts-cm-translate-target-system = Zielsystem
hts-cm-translate-target-system-placeholder = kanonische URL
hts-cm-translate-source-url = Quell-ValueSet
hts-cm-translate-source-url-placeholder = kanonische URL (optional)
hts-cm-translate-target-url = Ziel-ValueSet
hts-cm-translate-target-url-placeholder = kanonische URL (optional)
hts-cm-translate-date = Datum
hts-cm-translate-date-placeholder = ISO 8601 (z. B. 2025-06-01)
hts-cm-translate-submit = Übersetzen
hts-cm-translate-matches-count = { $count ->
    [one] { $count } Treffer
   *[other] { $count } Treffer
}
hts-cm-translate-no-matches = Keine Treffer für diese Quelle.
hts-cm-translate-column-code = Code
hts-cm-translate-column-system = System
hts-cm-translate-column-display = Anzeige
hts-cm-translate-column-mapping = { $kind ->
    [equivalence] Äquivalenz
    [relationship] Beziehung
   *[other] Mapping
}
hts-cm-translate-column-origin = Ursprung

## HTS Slice E -- Standalone-Operations-Workbench (design doc s7.6).






hts-vs-expand-advanced = Erweiterte Parameter
hts-vs-expand-total = Gesamt { $n }





## Slice F — Import (§7.7). Erste Uebersetzung; im i18n-Review pruefen
## (# TODO(F): review de).

hts-import-title = Terminologie importieren
hts-import-heading = Terminologie importieren
hts-import-help = Sende ein FHIR-Bundle als JSON. HTS akzeptiert CodeSystem, ValueSet und ConceptMap in einem POST.
hts-import-source-legend = Quelle
hts-import-source-paste = JSON einfuegen
hts-import-source-file = Datei hochladen
hts-import-bundle-textarea-label = FHIR-Bundle (JSON)
hts-import-bundle-file-label = Bundle-Datei (JSON)
hts-import-submit = Importieren
hts-import-status-empty = Es wurde noch kein Import gesendet.
hts-import-status-success = Import abgeschlossen
hts-import-status-partial = Import teilweise erfolgreich
hts-import-status-rejected = Import abgelehnt
hts-import-status-too-large = Bundle zu gross
hts-import-counts-heading = Anzahl pro Ressource
hts-import-counts-created = Erstellt / aktualisiert
hts-import-resource-code-system = CodeSystem
hts-import-resource-value-set = ValueSet
hts-import-resource-concept-map = ConceptMap
hts-import-resource-concept = Eingefuegte Konzepte
hts-import-issues-heading = { $n ->
    [one] { $n } Hinweis
   *[other] { $n } Hinweise
}
hts-import-too-large-hint = Die Anfrage hat das Serverlimit ueberschritten. Teile das Bundle in kleinere Batches auf und versuche es erneut.
hts-import-empty-bundle-error = Bitte ein JSON-Bundle einfuegen, bevor du absendest.
hts-import-invalid-json-error = Der uebermittelte Inhalt ist kein gueltiges JSON.

# Import in Schritten (V3, #551): Quelle waehlen, pruefen, Ergebnis.
# Schritt 2 zeigt bewusst keine Anzahlen: HTS liefert sie erst in der
# Antwort auf POST /import.
hts-import-step-source = Quelle waehlen
hts-import-step-review = Pruefen
hts-import-step-result = Ergebnis
hts-import-file-hint = Nur JSON. Die Datei wird im Browser gelesen und in das Bundle-Feld unten kopiert; gesendet wird erst beim Absenden.
hts-import-bundle-hint = Das Bundle wird an POST /import auf dem Terminologieserver gesendet. Vorhandene Ressourcen werden ueber url + version zugeordnet.
hts-import-review-target = Zielserver
hts-import-review-request = Anfrage
hts-import-review-accepted = Akzeptierte Ressourcen
hts-import-review-accepted-value = CodeSystem, ValueSet, ConceptMap
hts-import-review-existing = Vorhandene Ressourcen
hts-import-review-existing-value = Werden an Ort und Stelle aktualisiert, wenn url und version uebereinstimmen.
hts-import-review-note = Vor dem Absenden wird nichts geschrieben. Wie viele Ressourcen tatsaechlich angelegt wurden, meldet der Server im Ergebnis unten.
hts-import-counts-resource = Ressource
hts-import-raw-toggle = Rohantwort
hts-import-rejected-note = Es wurde nichts in den Terminologiespeicher geschrieben.
hts-import-tag-success = Erfolg
hts-import-tag-partial = Teilweise
hts-import-tag-error = Fehler

## Slice G — Diagnose (§7.9). Erste Uebersetzung; im i18n-Review pruefen
## (# TODO(G): review de).


# Konzept-Informationsebene (Richtung B, "Konzept zuerst").
# Das Konzept ist ein Objekt erster Ordnung mit eigenem Permalink unter
# /ui/hts/concepts?system=...&code=..., dargestellt in drei Panels:
# Identitaet, Zuordnungen (ueber alle gespeicherten ConceptMaps) und Subsumption.
hts-concept-title = Konzept
hts-concept-lede = Ein Code aus jedem Blickwinkel, den der Terminologieserver beantworten kann: was er ist, worauf er abgebildet wird und wo er in der Hierarchie steht.
hts-concept-open = Konzept öffnen
hts-concept-panel-loading = Wird geladen
hts-concept-panel-open = Dieses Panel öffnen

hts-concept-identity-heading = Identität
hts-concept-status-active = Aktiv
hts-concept-status-inactive = Inaktiv
hts-concept-status-unreported = Aktivität nicht gemeldet
hts-concept-field-system = System
hts-concept-field-code = Code
hts-concept-field-display = Anzeigetext
hts-concept-field-code-system-name = Name des CodeSystem
hts-concept-field-version = Version
hts-concept-field-selectability = Auswählbarkeit
hts-concept-selectability-abstract = Abstrakt (nicht auswählbar)
hts-concept-selectability-selectable = Auswählbar
hts-concept-field-definition = Definition
hts-concept-field-neighbours = Nachbarn in der Hierarchie
hts-concept-field-used-supplements = Angewandte Ergänzungen
hts-concept-designations-heading = Bezeichnungen
hts-concept-designations-value = Bezeichnung
hts-concept-designations-language = Sprache
hts-concept-designations-use = Verwendung
hts-concept-properties-heading = Eigenschaften
hts-concept-properties-code = Eigenschaft
hts-concept-properties-value = Wert
hts-concept-raw-response = Rohantwort

hts-concept-mappings-heading = Zuordnungen
hts-concept-mappings-direction-forward = Zuordnungen, in denen dieses Konzept die Quelle ist, über alle gespeicherten ConceptMaps hinweg.
hts-concept-mappings-direction-reverse = Zuordnungen, in denen dieses Konzept das Ziel ist, über alle gespeicherten ConceptMaps hinweg.
hts-concept-mappings-switch-forward = Zuordnungen von diesem Konzept anzeigen
hts-concept-mappings-switch-reverse = Zuordnungen auf dieses Konzept anzeigen
hts-concept-mappings-empty = Keine ConceptMap bildet dieses Konzept ab.
hts-concept-mappings-vocabulary = Zuordnungsvokabular
hts-concept-mappings-vocabulary-equivalence = equivalence (R4 / R4B)
hts-concept-mappings-vocabulary-relationship = relationship (R5 / R6)
hts-concept-mappings-vocabulary-unknown = Nicht gemeldet
hts-concept-mappings-unattributable = Der Server ordnet Treffer im Rückwärtsmodus keiner Quellzuordnung zu, daher lässt sich die Herkunft nicht anzeigen. Wechseln Sie in die Vorwärtsrichtung, um zu sehen, aus welcher ConceptMap jede Zuordnung stammt.
hts-concept-mappings-origin = Herkunftszuordnung
hts-concept-mappings-column-code = Code
hts-concept-mappings-column-system = System
hts-concept-mappings-column-display = Anzeigetext
hts-concept-mappings-column-mapping = Beziehung

hts-concept-relations-heading = Subsumption
hts-concept-relations-lede = Jede Zeile ist eine Subsumptionsprüfung. Der Vorfahrenkandidat wird immer als Code A gesendet, sodass eine in sich stimmige Hierarchie jedes Mal "subsumes" antwortet.
hts-concept-relation-parent = Übergeordnet
hts-concept-relation-child = Untergeordnet
hts-concept-relation-manual = Verglichen
hts-concept-relations-column-relation = Beziehung
hts-concept-relations-column-question = Gestellte Frage
hts-concept-relations-column-outcome = Ergebnis
hts-concept-relations-subsumes-verb = subsumiert
hts-concept-subsumes-outcome-equivalent = Gleichwertig
hts-concept-subsumes-outcome-subsumes = Subsumiert
hts-concept-subsumes-outcome-subsumed-by = Subsumiert von
hts-concept-subsumes-outcome-not-subsumed = Nicht subsumiert
hts-concept-relations-conflict-caveat = Die Konzeptabfrage meldet diese Hierarchiebeziehung, die Subsumptionsprüfung bestätigt sie jedoch nicht. Meist wurde die Subsumptionshülle nach dem erneuten Import des CodeSystem nicht neu aufgebaut; die Hierarchie selbst blieb erhalten.
hts-concept-relations-empty = Dieses Konzept hat keine über- oder untergeordneten Codes zum Vergleich.
hts-concept-relations-dropped = { $n } weitere Vergleichscodes wurden nicht geprüft; dieses Panel führt höchstens 20 Subsumptionsaufrufe pro Darstellung aus.
hts-concept-relations-compare-label = Mit Code vergleichen
hts-concept-relations-compare-placeholder = Ein anderer Code in diesem System
hts-concept-relations-compare-hint = Das System ist auf das dieses Konzepts festgelegt, geben Sie daher nur den Code ein. Geprüft wird, ob dieser Code das vorliegende subsumiert.
hts-concept-relations-compare-submit = Vergleichen

## HTS-Detailseiten -- kompakte Kopfzeile V3 (#551, Slices B/C/D).
## Gemeinsame Beschriftungen für die Chip-Zeile und die Aufklappbox der
## CodeSystem-/ValueSet-/ConceptMap-Detailseiten sowie die Überschriften
## der Ergebnispanels und die beiden Ehrlichkeitshinweise (Baummodus-Pager,
## originMap im Rückwärtsmodus).

hts-detail-facts-label = Fakten
hts-detail-canonical-url = Kanonische URL
hts-detail-version-label = Version
hts-detail-status-label = Status
hts-cs-detail-facts-summary = Alle CodeSystem-Fakten
hts-vs-detail-facts-summary = Alle ValueSet-Fakten
hts-cm-detail-facts-summary = Alle ConceptMap-Fakten
hts-cs-lookup-definition = Definition
hts-cs-validate-result-heading = Validierungsergebnis
hts-cs-subsumes-result-heading = Subsumtionsergebnis
hts-vs-expand-result-heading = Expansion
hts-vs-expand-table-caption = Vom Terminologieserver zurückgegebene Expansionsmitglieder.
hts-vs-expand-tree-note = Der Baummodus liefert die gesamte Hierarchie; der Pager gibt es nur im flachen Modus.
hts-cm-translate-table-caption = Vom Terminologieserver zurückgegebene Übersetzungstreffer.
hts-cm-translate-origin-reverse-note = Im Rückwärtsmodus lässt HTS originMap weg, daher kann ein Treffer keiner bestimmten ConceptMap zugeordnet werden. Jede Herkunftszelle bleibt bewusst ein Gedankenstrich – es fehlt kein Wert.


# Capability & Conformance page (HTS mirror of HFS's page). The shared
# `cap-*` and `nav-capability-conformance` keys carry everything both
# pages say identically; only what is specific to a terminology server
# lives here.
hts-capability-lede = Was dieser Terminologieserver aktuell anbietet — live aus /metadata zusammengestellt.
hts-capability-operations-empty = Keine Operationen angekündigt.
hts-capability-rest-empty = Keine REST-Ressourcen angekündigt.
hts-capability-terminology-heading = Terminologie-Fähigkeiten
hts-capability-expansion-hierarchical = Hierarchische Expansion
hts-capability-expansion-paging = Expansions-Paging
hts-capability-expansion-incomplete = Unvollständige Expansionen
hts-capability-expansion-parameters = $expand-Parameter
hts-capability-validate-code-translations = Validate-code-Übersetzungen
hts-capability-translation-needs-map = Übersetzung benötigt eine Map
hts-capability-closure = Closure-Pflege
hts-capability-code-systems-declared = Deklarierte Codesysteme
hts-capability-flag-true = Ja
hts-capability-flag-false = Nein
hts-capability-raw-truncated = Auf die ersten { $shown } von { $total } Bytes gekürzt — die Deklaration dieses Servers wächst mit den geladenen Codesystemen.
hts-capability-raw-full = Vollständige Deklaration ansehen

# Home V3 tile sub-lines. The mockup folds Backend, FHIR version,
# Bundled data and Avg latency into the sub-line of the tile each
# qualifies, instead of giving them tiles of their own.
hts-home-tile-status-sub = Backend { $backend } · FHIR { $version }
hts-home-tile-uptime-sub = hts v{ $version } · keine Neustarts seit { $since } UTC
hts-home-tile-uptime-sub-noclock = hts v{ $version }
hts-home-tile-loaded-systems-sub = { $mib } MiB auf der Festplatte gebündelt
hts-home-tile-requests-sub = { $ms } ms Durchschnitt · aus /metrics

# Chart caption, composed from the SELECTED window and status class.
# Each locale owns its own word order through the two placeables.
hts-home-chart-hint = { $window }, { $classes }. Wird erfasst, solange diese Seite geöffnet ist. Ohne den eigenen 15-s-Aktualisierungsabruf dieser Seite und /metrics.
hts-home-chart-hint-window-15m = Letzte 15 Minuten
hts-home-chart-hint-window-1h = Letzte Stunde
hts-home-chart-hint-window-6h = Letzte 6 Stunden
hts-home-chart-hint-series-all = alle Statusklassen
hts-home-chart-hint-series-2xx = nur 2xx-Antworten
hts-home-chart-hint-series-4xx = nur 4xx-Antworten
hts-home-chart-hint-series-5xx = nur 5xx-Antworten
