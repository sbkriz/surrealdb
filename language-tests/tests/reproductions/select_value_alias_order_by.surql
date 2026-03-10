/**
[test]
reason = "SELECT VALUE with AS alias should resolve alias in ORDER BY"

[[test.results]]
value = "'OK'"

[[test.results]]
value = "[{ v_alias: 0 }, { v_alias: 1 }, { v_alias: 2 }, { v_alias: 3 }, { v_alias: 4 }]"

[[test.results]]
value = "[0, 1, 2, 3, 4]"

*/
{
    CREATE foo set v = 0;
    CREATE foo set v = 1;
    CREATE foo set v = 2;
    CREATE foo set v = 3;
    CREATE foo set v = 4;
    CREATE foo set v = 5;
    CREATE foo set v = 6;
    CREATE foo set v = 7;
    CREATE foo set v = 8;
    CREATE foo set v = 9;
    "OK"
};
SELECT v as v_alias FROM foo ORDER BY v_alias LIMIT 5;
SELECT value v as v_alias FROM foo ORDER BY v_alias LIMIT 5;
