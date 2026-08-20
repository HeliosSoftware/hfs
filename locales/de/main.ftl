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
nav-import = Importieren
nav-export = Exportieren
nav-sql-on-fhir = SQL-on-FHIR
nav-capability-conformance = Capability & Konformität
nav-search-parameters = Suchparameter
nav-admin-ops = Admin / Betrieb
nav-subscriptions = Abonnements
nav-tenants = Mandanten

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
tenants-row-provisioning = Wird bereitgestellt … das kann einen Moment dauern.
tenants-row-failed = Bereitstellung fehlgeschlagen
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
card-resource-types-sub = aktiviert für { $version }
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
chart-remove-series = Aus dem Diagramm entfernen

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
crud-delete-failed = Löschen fehlgeschlagen

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
plain-and = und
plain-or = oder
plain-arrow = {" "}→
plain-has = die ein verknüpftes {"{type}"} haben, dessen {"{param}"} {"{verb}"} {"{value}"}
plain-include = Zusätzlich wird der {"{param}"} jedes {"{type}"} zurückgegeben{"{target}"}
plain-revinclude = Plus jedes {"{type}"}, dessen {"{param}"} hierher zeigt
plain-iterate = (wiederholt)
plain-count = Zeigt {"{n}"} pro Seite
plain-sort = Sortiert nach {"{sort}"}
plain-verb-is = ist
plain-verb-contains = enthält
plain-verb-exact = ist genau
plain-verb-missing = ist vorhanden/fehlt
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
editor-edit-raw = Rohtext bearbeiten
editor-versions = Versionen
editor-versions-none = Keine früheren Versionen.
## Verlauf & Versionen (#236)

## Ressourcen-Arbeitsbereich (#282)

resources-heading = Ressourcen
resources-lede = FHIR-Ressourcen durchsuchen, suchen, erstellen und bearbeiten. In natürlicher Sprache suchen oder die Abfrage selbst bauen, dann ein Ergebnis zum Bearbeiten öffnen.
resources-create = Neu erstellen
resources-save-blocked = Beheben Sie die Validierungsprobleme vor dem Speichern.
resources-save-invalid = Das JSON ist ungültig — beheben Sie es vor dem Speichern.
resources-edit-title = Ressource bearbeiten
resources-tab-edit = Bearbeiten
resources-tab-history = Verlauf
resources-types-heading = Ressourcentypen

queries-saved-group = Gespeichert

nav-collapse = Menü einklappen

batch-heading = Batch / Transaction
batch-lede = Lade ein FHIR-Bundle hoch, prüfe die auszuführenden Aktionen, führe es gegen diesen Server aus und lies das Ergebnis jedes Eintrags.
batch-upload = Hochladen
batch-drop-hint = Bundle-JSON-Datei hier ablegen
batch-drop-browse = oder klicken zum Durchsuchen
batch-invalid-json = Diese Datei ist kein gültiges JSON
batch-not-a-bundle = Dieses JSON ist kein FHIR-Bundle
batch-bad-type = Hier lassen sich nur Bundles vom Typ batch oder transaction ausführen
batch-request = Anfrage
batch-entries = Einträge
batch-semantics-batch = Batch: Einträge laufen unabhängig — ein fehlgeschlagener Eintrag stoppt die anderen nicht und macht sie nicht rückgängig.
batch-semantics-transaction = Transaction: alles oder nichts — schlägt ein Eintrag fehl, rollt der Server das gesamte Bundle zurück.
batch-tab-actions = Aktionen
batch-tab-json = Bundle-JSON
batch-no-body = (kein Body — dieser Eintrag adressiert nur eine Ressource)
batch-cancel = Abbrechen
batch-upload-another = Weitere hochladen
batch-execute = Ausführen
batch-response-heading = Ergebnisse pro Aktion
batch-sum-created = erstellt
batch-sum-updated = aktualisiert
batch-sum-other = gelesen/sonstige
batch-sum-failed = fehlgeschlagen
batch-request-failed = Die Anfrage ist fehlgeschlagen
batch-back = Zurück zum Bundle
batch-execute-again = Erneut ausführen

## Bulk Import workspace (#527)

bulk-import-title = Massenimport
bulk-import-new = Neue Submission
bulk-import-create-title = Bulk Submission anlegen
bulk-import-field-name = Name der Submission
bulk-import-field-recipient = Basis-URL des Empfängers
bulk-import-field-recipient-hint = Die Basis-URL des Servers, an den die Daten übermittelt werden.
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
bulk-import-create-submit = Submission anlegen
bulk-import-unavailable = Das Storage-Backend hostet keinen Settings-Store; Submissions können nicht gespeichert werden.
bulk-import-submissions = Submissions
bulk-import-records = Einträge
bulk-import-col-name = Name
bulk-import-col-status = Status
bulk-import-col-created = Erstellt
bulk-import-col-manifests = Manifeste
bulk-import-col-destination = Ziel
bulk-import-empty = Noch keine Submissions. Legen Sie eine an, um zu beginnen.
bulk-import-all = Alle Submissions
bulk-import-status-not-started = Nicht gestartet
bulk-import-status-in-progress = In Bearbeitung
bulk-import-status-stopped = Angehalten
bulk-import-status-completed = Abgeschlossen
bulk-import-detail-recipient = Datenempfänger
bulk-import-detail-id = Submission-ID
bulk-import-detail-submitter = Einreicher
bulk-import-detail-created = Erstellt
bulk-import-detail-status = Status
bulk-import-detail-auth = Authentifizierung
bulk-import-abort = Abbrechen
bulk-import-complete = Abschließen
bulk-import-delete = Löschen
bulk-import-add-manifest = Manifest hinzufügen
bulk-import-add-manifest-title = Manifest hinzufügen
bulk-import-add-manifest-submit = Hinzufügen
bulk-import-field-manifest-url = Manifest-URL
bulk-import-field-manifest-url-hint = URL eines Bulk-Export-Manifests mit einem vorkoordinierten FHIR-Datensatz.
bulk-import-field-fhir-base = FHIR-Basis-URL
bulk-import-field-fhir-base-hint = Basis-URL, die der Empfänger beim Auflösen relativer Referenzen verwendet. Leer lassen, um die Basis-URL des Manifests zu verwenden.
bulk-import-field-output-format = Ausgabeformat
bulk-import-field-output-format-hint = Das Format der Bulk-Data-Dateien im Manifest.
bulk-import-field-headers = Header für Dateiabrufe
bulk-import-field-headers-hint = HTTP-Header, die der Empfänger beim Abruf einer Datendatei verwenden soll, je Zeile "Name: Wert".
bulk-import-manifests = Manifeste
bulk-import-no-manifests = Noch keine Manifeste. Fügen Sie eines hinzu, um Daten zu übermitteln.
bulk-import-submit = Übermitteln
bulk-import-submit-all = Alle übermitteln
bulk-import-remove = Entfernen
bulk-import-log = Submission-Protokoll
bulk-import-log-empty = Noch nichts übermittelt.
bulk-import-field-submitter-system = Einreicher-System
bulk-import-field-submitter-value = Einreicher-Wert
bulk-import-field-submitter-hint = Muss einem beim Empfänger registrierten Identifier entsprechen (außerhalb des Protokolls abgestimmt). Leer lassen für die generierten Standardwerte.
bulk-import-field-submission-id = Submission-ID
bulk-import-field-submission-id-hint = Eindeutig je Einreicher. Leer lassen, um eine UUID zu generieren.
bulk-import-processing = Verarbeitung
bulk-import-processing-waiting = Warten auf den ersten Statusbericht des Empfängers …
bulk-import-result = Ergebnis
bulk-import-result-finished = Verarbeitung abgeschlossen um
bulk-import-result-outputs = Ausgabedateien
bulk-import-result-errors = Fehlerdateien
bulk-import-abort-manifest = Abbrechen
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
subs-empty = Für diesen Mandanten sind keine Abonnements registriert.
subs-card-failing = Fehlgeschlagen
subs-card-failing-sub = braucht Aufmerksamkeit
subs-card-idle = Inaktiv
subs-card-idle-sub = keine Clients
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
subs-state-active = aktiv
subs-state-error = Fehler
subs-state-idle = 0 Clients
subs-state-requested = angefragt
subs-state-off = aus

## Bulk Export workspace (#537)

bulk-export-title = Massenexport
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
