//! Elasticsearch Query DSL builder.
//!
//! Translates FHIR `SearchQuery` into Elasticsearch Query DSL JSON.

use serde_json::{Value, json};

use crate::types::{
    PageCursor, SearchModifier, SearchParamType, SearchParameter, SearchPrefix, SearchQuery,
    SortDirection, SortDirective,
};

use super::fts;
use super::modifier_handlers;
use super::parameter_handlers::{composite, date, number, quantity, reference, string, token, uri};

/// A complete Elasticsearch query body ready to be sent.
#[derive(Debug, Clone)]
pub struct EsQuery {
    /// The complete query body.
    pub body: Value,
    /// The index to search.
    pub index: String,
}

/// Builds Elasticsearch queries from FHIR search queries.
pub struct EsQueryBuilder<'a> {
    tenant_id: &'a str,
    #[allow(dead_code)]
    resource_type: &'a str,
    index: String,
}

impl<'a> EsQueryBuilder<'a> {
    /// Creates a new query builder.
    pub fn new(tenant_id: &'a str, resource_type: &'a str, index: String) -> Self {
        Self {
            tenant_id,
            resource_type,
            index,
        }
    }

    /// Builds a complete ES query from a FHIR SearchQuery.
    pub fn build(&self, query: &SearchQuery) -> EsQuery {
        let mut must_clauses: Vec<Value> = Vec::new();
        let filter_clauses: Vec<Value> = vec![
            json!({ "term": { "tenant_id": self.tenant_id } }),
            json!({ "term": { "is_deleted": false } }),
        ];

        // Process each search parameter
        for param in &query.parameters {
            if let Some(clause) = self.build_parameter_clause(param) {
                must_clauses.push(clause);
            }
        }

        // Build the bool query
        let mut bool_query = json!({
            "filter": filter_clauses,
        });

        if !must_clauses.is_empty() {
            bool_query["must"] = json!(must_clauses);
        }

        let mut body = json!({
            "query": { "bool": bool_query },
        });

        // Add sorting
        let sort = self.build_sort(&query.sort);
        body["sort"] = sort;

        // Add pagination
        let count = query.count.unwrap_or(20);
        body["size"] = json!(count);

        if let Some(ref cursor_str) = query.cursor {
            if let Ok(cursor) = PageCursor::decode(cursor_str) {
                let search_after = self.build_search_after(&cursor);
                body["search_after"] = search_after;
            }
        } else if let Some(offset) = query.offset {
            body["from"] = json!(offset);
        }

        // Track total hits
        body["track_total_hits"] = json!(true);

        EsQuery {
            body,
            index: self.index.clone(),
        }
    }

    /// Builds a clause for a single search parameter.
    fn build_parameter_clause(&self, param: &SearchParameter) -> Option<Value> {
        // Handle special parameters
        match param.name.as_str() {
            "_id" => return self.build_id_clause(param),
            "_lastUpdated" => return self.build_last_updated_clause(param),
            "_text" => return fts::build_text_clause(param),
            "_content" => return fts::build_content_clause(param),
            _ => {}
        }

        // Handle :missing modifier
        if param.modifier == Some(SearchModifier::Missing) {
            return modifier_handlers::build_missing_clause(param);
        }

        // Dispatch based on parameter type
        let clauses: Vec<Value> = param
            .values
            .iter()
            .filter_map(|value| self.build_value_clause(param, &value.value, value.prefix))
            .collect();

        if clauses.is_empty() {
            return None;
        }

        // Multiple values for the same parameter are ORed
        if clauses.len() == 1 {
            Some(clauses.into_iter().next().unwrap())
        } else {
            Some(json!({
                "bool": {
                    "should": clauses,
                    "minimum_should_match": 1
                }
            }))
        }
    }

    /// Builds a clause for a single value of a parameter.
    fn build_value_clause(
        &self,
        param: &SearchParameter,
        value: &str,
        prefix: SearchPrefix,
    ) -> Option<Value> {
        match param.param_type {
            SearchParamType::String => string::build_clause(param, value),
            SearchParamType::Token => token::build_clause(param, value),
            SearchParamType::Date => date::build_clause(&param.name, value, prefix),
            SearchParamType::Number => number::build_clause(&param.name, value, prefix),
            SearchParamType::Quantity => quantity::build_clause(&param.name, value, prefix),
            SearchParamType::Reference => reference::build_clause(param, value),
            SearchParamType::Uri => uri::build_clause(param, value),
            SearchParamType::Composite => composite::build_clause(param, value),
            SearchParamType::Special => None,
        }
    }

    /// Builds a clause for the _id special parameter.
    fn build_id_clause(&self, param: &SearchParameter) -> Option<Value> {
        let ids: Vec<&str> = param.values.iter().map(|v| v.value.as_str()).collect();
        if ids.len() == 1 {
            Some(json!({ "term": { "resource_id": ids[0] } }))
        } else {
            Some(json!({ "terms": { "resource_id": ids } }))
        }
    }

    /// Builds a clause for the _lastUpdated special parameter.
    fn build_last_updated_clause(&self, param: &SearchParameter) -> Option<Value> {
        let mut range = serde_json::Map::new();
        for value in &param.values {
            match value.prefix {
                SearchPrefix::Eq => {
                    range.insert("gte".to_string(), json!(value.value));
                    range.insert("lte".to_string(), json!(value.value));
                }
                SearchPrefix::Gt => {
                    range.insert("gt".to_string(), json!(value.value));
                }
                SearchPrefix::Lt => {
                    range.insert("lt".to_string(), json!(value.value));
                }
                SearchPrefix::Ge => {
                    range.insert("gte".to_string(), json!(value.value));
                }
                SearchPrefix::Le => {
                    range.insert("lte".to_string(), json!(value.value));
                }
                _ => {
                    range.insert("gte".to_string(), json!(value.value));
                    range.insert("lte".to_string(), json!(value.value));
                }
            }
        }
        if range.is_empty() {
            None
        } else {
            Some(json!({ "range": { "last_updated": Value::Object(range) } }))
        }
    }

    /// Builds the sort clause.
    fn build_sort(&self, directives: &[SortDirective]) -> Value {
        if directives.is_empty() {
            // Default sort: _lastUpdated descending, then _id for tie-breaking
            return json!([
                { "last_updated": { "order": "desc" } },
                { "resource_id": { "order": "asc" } }
            ]);
        }

        let mut sort_clauses: Vec<Value> = Vec::new();

        for directive in directives {
            let order = match directive.direction {
                SortDirection::Ascending => "asc",
                SortDirection::Descending => "desc",
            };

            match directive.parameter.as_str() {
                "_id" => {
                    sort_clauses.push(json!({ "resource_id": { "order": order } }));
                }
                "_lastUpdated" => {
                    sort_clauses.push(json!({ "last_updated": { "order": order } }));
                }
                // For other parameters, sort on the nested search_params field
                name => {
                    // Use nested sort on the most likely field type (string)
                    sort_clauses.push(json!({
                        "search_params.string.value.keyword": {
                            "order": order,
                            "nested": {
                                "path": "search_params.string",
                                "filter": {
                                    "term": { "search_params.string.name": name }
                                }
                            },
                            "missing": if order == "asc" { "_last" } else { "_first" }
                        }
                    }));
                }
            }
        }

        // Always add tie-breaker
        sort_clauses.push(json!({ "resource_id": { "order": "asc" } }));

        Value::Array(sort_clauses)
    }

    /// Builds the search_after clause from a cursor.
    fn build_search_after(&self, cursor: &PageCursor) -> Value {
        let mut values: Vec<Value> = cursor
            .sort_values()
            .iter()
            .map(|v| match v {
                crate::types::CursorValue::String(s) => json!(s),
                crate::types::CursorValue::Number(n) => json!(n),
                crate::types::CursorValue::Decimal(d) => json!(d.to_string()),
                crate::types::CursorValue::Boolean(b) => json!(b),
                crate::types::CursorValue::Null => json!(null),
            })
            .collect();

        // Append the resource_id tie-breaker
        values.push(json!(cursor.resource_id()));

        Value::Array(values)
    }
}

/// Builds a count query (no sorting, no source, size=0).
pub fn build_count_query(tenant_id: &str, resource_type: &str, query: &SearchQuery) -> Value {
    let builder = EsQueryBuilder::new(tenant_id, resource_type, String::new());
    let es_query = builder.build(query);

    // Strip unnecessary fields for count
    let mut body = es_query.body;
    if let Some(obj) = body.as_object_mut() {
        obj.remove("sort");
        obj.remove("size");
        obj.remove("from");
        obj.remove("search_after");
    }
    body["size"] = json!(0);

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchValue, SortDirection};

    #[test]
    fn test_basic_query_build() {
        let query = SearchQuery::new("Patient");
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query);

        // Should have tenant and is_deleted filters
        let filters = &es_query.body["query"]["bool"]["filter"];
        assert!(filters.is_array());
    }

    #[test]
    fn test_id_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("123")],
            chain: vec![],
            components: vec![],
        });

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();
        assert!(body_str.contains("resource_id"));
    }

    #[test]
    fn test_default_sort() {
        let query = SearchQuery::new("Patient");
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query);

        let sort = &es_query.body["sort"];
        assert!(sort.is_array());
        assert!(sort[0]["last_updated"]["order"].as_str() == Some("desc"));
    }

    #[test]
    fn test_custom_sort() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "_id".to_string(),
            direction: SortDirection::Ascending,
            param_type: None,
        });

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query);

        let sort = &es_query.body["sort"];
        assert!(sort[0]["resource_id"]["order"].as_str() == Some("asc"));
    }
}
