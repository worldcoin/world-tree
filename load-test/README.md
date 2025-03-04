# Tree Availability Service Load Test

This load test uses [k6](https://github.com/grafana/k6) to evaluate the performance of the `tree-availabiliy-service` under high stress. To run the load test, first make sure that you have `k6` installed. Next, you will need to start a new instance of the `tree-availability-service` with the following command.

```
cargo run --bin tree-availability-service -- --tree-depth <depth> --tree-history-size <history_size> -a <world_tree_address> -c <world_tree_creation_block> -r <rpc_endpoint> -t <requests_per_minute>
```

Once the `tree-availability-service` is synced, you can start the load test with the command below. You can use any duration for the test.

```
k6 run --vus <num_virtual_users> --duration 300s  -e TREE_AVAILABILITY_SERVICE_ENDPOINT=<endpoint> load-test/loadTest.js
```

