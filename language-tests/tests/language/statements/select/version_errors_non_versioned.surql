/**
[env]
backend = ["mem", "surrealkv"]
versioned = false
timeout = 5000

[test]
reason = "VERSION clause on a non-versioned backend should produce an error"

[[test.results]]
value = "[{ id: test:1, val: 1 }]"

[[test.results]]
error = true
*/

-- Create some data
CREATE test:1 SET val = 1;

-- Querying with VERSION on a non-versioned backend should fail
SELECT * FROM test VERSION d'2025-01-01T00:00:00Z';
