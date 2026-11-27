
### PREWARM

| metric | better | HFS R1 | R2 | R3 | HFS median | spread% | WSO2 R1 | R2 | R3 | WSO2 median | spread% | ratio HFS/WSO2 | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| HTTP req/s | higher | 2137 | 2033 | 1728 | 2033 | 20.1 | 1826 | 1857 | 1531 | 1826 | 17.8 | 1.11 | within noise (ranges overlap) |
| latency avg ms | lower | 4.48 | 4.70 | 5.55 | 4.70 | 22.8 | 5.26 | 5.16 | 6.30 | 5.26 | 21.7 | 0.89 | within noise (ranges overlap) |
| latency p50 ms | lower | 4.94 | 5.19 | 5.54 | 5.19 | 11.6 | 5.46 | 5.39 | 5.48 | 5.46 | 1.6 | 0.95 | within noise (ranges overlap) ~ |
| latency p90 ms | lower | 7.20 | 7.46 | 9.97 | 7.46 | 37.1 | 8.29 | 8.17 | 8.73 | 8.29 | 6.8 | 0.90 | within noise (ranges overlap) |
| latency p95 ms | lower | 7.79 | 8.07 | 10.70 | 8.07 | 36.1 | 9.26 | 9.12 | 10.49 | 9.26 | 14.8 | 0.87 | within noise (ranges overlap) |
| latency max ms | lower | 76 | 224 | 78 | 78 | 190.1 | 65 | 49 | 72 | 65 | 35.8 | 1.21 | HFS worse (ranges do not overlap) |
| iterations/s | higher | 59.4 | 56.5 | 48.0 | 56.5 | 20.1 | 50.7 | 51.6 | 42.5 | 50.7 | 17.8 | 1.11 | within noise (ranges overlap) |

checks_failed across all 6 runs: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]

### CRUD

| metric | better | HFS R1 | R2 | R3 | HFS median | spread% | WSO2 R1 | R2 | R3 | WSO2 median | spread% | ratio HFS/WSO2 | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| HTTP req/s | higher | 2567 | 2564 | 2847 | 2567 | 11.0 | 2793 | 2825 | 2572 | 2793 | 9.0 | 0.92 | within noise (ranges overlap) |
| iterations/s | higher | 71.3 | 71.2 | 79.1 | 71.3 | 11.0 | 77.6 | 78.5 | 71.5 | 77.6 | 9.0 | 0.92 | within noise (ranges overlap) |
| latency avg ms | lower | 116.2 | 116.5 | 104.9 | 116.2 | 10.0 | 106.9 | 105.6 | 116.0 | 106.9 | 9.7 | 1.09 | within noise (ranges overlap) |
| latency p50 ms | lower | 101.9 | 102.5 | 93.7 | 101.9 | 8.6 | 99.5 | 95.3 | 101.0 | 99.5 | 5.8 | 1.02 | within noise (ranges overlap) ~ |
| latency p90 ms | lower | 190.6 | 195.4 | 166.5 | 190.6 | 15.1 | 169.9 | 176.5 | 198.5 | 176.5 | 16.2 | 1.08 | within noise (ranges overlap) |
| latency p95 ms | lower | 234.3 | 226.6 | 207.1 | 226.6 | 12.0 | 191.2 | 205.0 | 233.4 | 205.0 | 20.6 | 1.11 | within noise (ranges overlap) |
| latency max ms | lower | 5760 | 2970 | 691 | 2970 | 170.7 | 832 | 1250 | 1400 | 1250 | 45.4 | 2.38 | within noise (ranges overlap) |
| HTTP failed % | lower | 0.00 | 0.00 | 0.00 | 0.00 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0.0 | nan | tie (0 failures) |

checks_failed across all 6 runs: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]

### IMPORT

| metric | better | HFS R1 | R2 | R3 | HFS median | spread% | WSO2 R1 | R2 | R3 | WSO2 median | spread% | ratio HFS/WSO2 | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| wall time s (1000 bundles) | lower | 643 | 736 | 750 | 736 | 14.5 | 814 | 868 | 833 | 833 | 6.4 | 0.88 | HFS better (ranges do not overlap) |
| iterations/s | higher | 1.554 | 1.358 | 1.332 | 1.358 | 16.3 | 1.228 | 1.152 | 1.200 | 1.200 | 6.3 | 1.13 | HFS better (ranges do not overlap) |
| latency avg ms | lower | 5440 | 6340 | 6560 | 6340 | 17.7 | 6670 | 7320 | 7000 | 7000 | 9.3 | 0.91 | HFS better (ranges do not overlap) |
| latency p50 ms | lower | 683 | 832 | 713 | 713 | 20.9 | 828 | 1070 | 861 | 861 | 28.1 | 0.83 | within noise (ranges overlap) |
| latency p90 ms | lower | 11130 | 13550 | 14820 | 13550 | 27.2 | 13330 | 16290 | 14920 | 14920 | 19.8 | 0.91 | within noise (ranges overlap) |
| latency p95 ms | lower | 21420 | 25410 | 27030 | 25410 | 22.1 | 27530 | 32000 | 29850 | 29850 | 15.0 | 0.85 | HFS better (ranges do not overlap) |
| latency max ms | lower | 313000 | 274000 | 300000 | 300000 | 13.0 | 275000 | 293000 | 264000 | 275000 | 10.5 | 1.09 | within noise (ranges overlap) |
| HTTP failed % | lower | 0.00 | 0.00 | 0.00 | 0.00 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0.0 | nan | tie (0 failures) |

checks_failed across all 6 runs: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]

### SEARCH

| metric | better | HFS R1 | R2 | R3 | HFS median | spread% | WSO2 R1 | R2 | R3 | WSO2 median | spread% | ratio HFS/WSO2 | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| HTTP req/s | higher | 2583 | 2646 | 2650 | 2646 | 2.6 | 2137 | 2464 | 2273 | 2273 | 14.4 | 1.16 | HFS better (ranges do not overlap) |
| iterations/s | higher | 107.6 | 110.2 | 110.4 | 110.2 | 2.6 | 89.1 | 102.7 | 94.7 | 94.7 | 14.4 | 1.16 | HFS better (ranges do not overlap) |
| latency avg ms | lower | 11.40 | 11.11 | 11.10 | 11.11 | 2.7 | 13.80 | 11.96 | 12.96 | 12.96 | 14.2 | 0.86 | HFS better (ranges do not overlap) |
| latency p50 ms | lower | 7.91 | 7.58 | 7.99 | 7.91 | 5.2 | 6.56 | 6.17 | 6.36 | 6.36 | 6.1 | 1.24 | HFS worse (ranges do not overlap) |
| latency p90 ms | lower | 25.75 | 23.76 | 24.39 | 24.39 | 8.2 | 35.23 | 34.34 | 36.22 | 35.23 | 5.3 | 0.69 | HFS better (ranges do not overlap) |
| latency p95 ms | lower | 34.40 | 31.66 | 31.87 | 31.87 | 8.6 | 44.84 | 44.04 | 45.79 | 44.84 | 3.9 | 0.71 | HFS better (ranges do not overlap) |
| latency max ms | lower | 1790 | 5590 | 1800 | 1800 | 211.1 | 13450 | 2300 | 2690 | 2690 | 414.5 | 0.67 | within noise (ranges overlap) |
| HTTP failed % | lower | 0.00 | 0.00 | 0.00 | 0.00 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0.0 | nan | tie (0 failures) |

checks_failed across all 6 runs: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
