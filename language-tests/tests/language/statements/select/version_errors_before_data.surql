/**
[env]
backend = ["mem", "surrealkv"]
versioned = true
timeout = 5000

[test]
reason = "VERSION clause with timestamp before any data existed should return empty, not error"

[[test.results]]
value = "[{ id: test:1, val: 1 }]"

[[test.results]]
value = "[]"

[[test.results]]
value = "[]"

[[test.results]]
value = "{ accesses: {}, apis: {}, analyzers: {}, buckets: {}, configs: {}, functions: {}, models: {}, modules: {}, params: {}, sequences: {}, tables: {}, users: {} }"
*/

-- Create some data
CREATE test:1 SET val = 1;

-- VERSION before any data existed: should return empty, not error
SELECT * FROM test VERSION d'1970-01-01T00:00:01Z';

-- Non-existent table at historical timestamp
SELECT * FROM nonexistent VERSION d'1970-01-01T00:00:01Z';

-- INFO FOR DB at epoch: should show empty schema
INFO FOR DB VERSION d'1970-01-01T00:00:01Z';
