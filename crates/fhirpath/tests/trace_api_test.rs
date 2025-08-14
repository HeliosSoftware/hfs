#[cfg(test)]
mod tests {
    use axum::{Json, response::Response};
    use helios_fhirpath::handlers::evaluate_fhirpath;
    use helios_fhirpath::models::FhirPathParameters;
    use serde_json::json;

    #[tokio::test]
    async fn test_trace_output_in_response() {
        // Create the test parameters with trace expression
        let params_json = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "expression",
                    "valueString": "trace('trc').given.join(' ')"
                },
                {
                    "name": "context",
                    "valueString": "name"
                },
                {
                    "name": "resource",
                    "resource": {
                        "resourceType": "Patient",
                        "id": "example",
                        "name": [{
                            "family": "Chalmers",
                            "given": ["Peter", "James"]
                        }]
                    }
                }
            ]
        });

        // Convert to FhirPathParameters
        let params: FhirPathParameters =
            serde_json::from_value(params_json).expect("Failed to parse parameters");

        // Call the handler
        let response = evaluate_fhirpath(Json(params)).await;

        // Check that we got a successful response
        assert!(response.is_ok());

        // Extract the response
        let response: Response = response.unwrap();

        // Check status
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        // Extract body
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read body");
        let body: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Failed to parse JSON");

        println!("Response: {}", serde_json::to_string_pretty(&body).unwrap());

        // Check that trace outputs are included
        let results = body["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"] == "result")
            .collect::<Vec<_>>();

        assert!(!results.is_empty(), "Should have at least one result");

        // Check the first result has trace parts
        let first_result = &results[0];
        let parts = first_result["part"].as_array().unwrap();

        // Find trace parts
        let trace_parts: Vec<_> = parts.iter().filter(|p| p["name"] == "trace").collect();

        assert!(!trace_parts.is_empty(), "Should have trace outputs");

        // Check the trace content
        let first_trace = &trace_parts[0];
        assert_eq!(first_trace["valueString"], "trc");
        assert!(first_trace["part"].is_array());

        // The traced value should be a HumanName
        let trace_value_parts = first_trace["part"].as_array().unwrap();
        assert!(!trace_value_parts.is_empty());
    }

    #[tokio::test]
    async fn test_trace_with_projection() {
        // Create the test parameters with trace projection
        let params_json = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "expression",
                    "valueString": "(1 | 2 | 3).trace('numbers', $this + 10)"
                },
                {
                    "name": "resource",
                    "resource": {
                        "resourceType": "Patient",
                        "id": "example"
                    }
                }
            ]
        });

        let params: FhirPathParameters =
            serde_json::from_value(params_json).expect("Failed to parse parameters");

        // Call the handler
        let response = evaluate_fhirpath(Json(params)).await;

        assert!(response.is_ok());

        let response: Response = response.unwrap();

        // Check status
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        // Extract body
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read body");
        let body: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Failed to parse JSON");

        println!(
            "Response with projection: {}",
            serde_json::to_string_pretty(&body).unwrap()
        );

        // Check trace outputs
        let results = body["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"] == "result")
            .collect::<Vec<_>>();

        let first_result = &results[0];
        let parts = first_result["part"].as_array().unwrap();

        let trace_parts: Vec<_> = parts.iter().filter(|p| p["name"] == "trace").collect();

        assert!(!trace_parts.is_empty(), "Should have trace outputs");

        let first_trace = &trace_parts[0];
        assert_eq!(first_trace["valueString"], "numbers");

        // Check that projection values are traced (11, 12, 13)
        let trace_values = first_trace["part"].as_array().unwrap();
        assert_eq!(trace_values.len(), 3);

        // Verify the traced projection values
        let values: Vec<i64> = trace_values
            .iter()
            .filter_map(|v| v["valueInteger"].as_i64())
            .collect();
        assert_eq!(values, vec![11, 12, 13]);
    }
}
