io_uring toy http echo server

Benchmarks:
```
wrk -d 5s -t 8 -c 200  http://127.0.0.1:9000/
Running 5s test @ http://127.0.0.1:9000/
  8 threads and 200 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency     1.28ms    6.77ms 218.73ms   99.57%
    Req/Sec    26.67k     2.83k   46.02k    88.89%
  1074578 requests in 5.10s, 90.18MB read
Requests/sec: 210726.98
Transfer/sec:     17.68MB
```
