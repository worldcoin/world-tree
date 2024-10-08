To verify that the world tree service and signup sequencer are in sync we can run a consistency check.

On a very basic level this is as simple as running
```
./utils consistency
```

This will fetch inclusion proofs for a set of randomly selected hardcoded identities. Correct output should look like this:
```
================================================= 39/39 [00:00:02]Matches: 39
Mismatches: 0
Missing: 0
Failures: 0
```

# With DB access
If you have access to the sequencer database you can run a more thorough consistency check by fetching a new set of identities.

To do that connect to the database and fetch new identities, assuming you're connecting to a localhost db the command would look like this
```
psql postgresql://postgres:postgres@localhost:5432/db  -c "WITH latest_leaf_id as (SELECT MAX(id) as id, leaf_index FROM identities GROUP BY leaf_index) SELECT encode(commitment, 'hex') FROM identities INNER JOIN latest_leaf_id ON latest_leaf_id.id = identities.id WHERE commitment <> '\x0000000000000000000000000000000000000000000000000000000000000000' AND status = 'mined' ORDER BY RANDOM() LIMIT 2400" -t -A > identities
```
then run the consistency check using this newly created file.

## Note
Make sure to include `commitment <> '\x0000000000000000000000000000000000000000000000000000000000000000'` in the query, otherwise the output will try to fetch inclusion proofs for zero identities which is invalid.

If you omit `status = 'mined'` from the query the output might include fresh identities which haven't been mined yet - these will identities will be present in the sequncer but not known yet to the world-tree service. But it's a useful test to run anyway, as running the same check in ~1-2 hours should match all identities.
