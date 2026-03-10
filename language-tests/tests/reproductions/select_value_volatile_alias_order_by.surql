/**
[test]
reason = "SELECT VALUE with volatile alias must not re-evaluate during projection"

[[test.results]]
value = "'OK'"

[[test.results]]
value = "true"

[[test.results]]
value = "true"

*/
{
    CREATE foo:1 SET v = 1;
    CREATE foo:2 SET v = 2;
    CREATE foo:3 SET v = 3;
    CREATE foo:4 SET v = 4;
    CREATE foo:5 SET v = 5;
    "OK"
};
-- Use blocks so the inner LET+SELECT goes through the planner path
{
    LET $asc = SELECT VALUE rand() AS r FROM foo ORDER BY r ASC;
    LET $asc_sorted = array::sort::asc($asc);
    RETURN $asc = $asc_sorted;
};
{
    LET $desc = SELECT VALUE rand() AS r FROM foo ORDER BY r DESC;
    LET $desc_sorted = array::sort::desc($desc);
    RETURN $desc = $desc_sorted;
};
