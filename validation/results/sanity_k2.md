Settings: k=2, p=0.9, 50000 nodes, Stockfish 19, maia3-5m.history.fp32.onnx@f2fc76df6749

| Position | Category | v* (W) | T_W 1100 | T_W 1500 | T_W 1900 | T_B 1100 | T_B 1500 | T_B 1900 | D 1100 | D 1500 | D 1900 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rr_1 | rook_vs_rook | 0.50 | +0.001 | +0.001 | +0.000 | +0.000 | +0.000 | +0.000 | 0.683 | 0.592 | 0.558 |
| rr_2 | rook_vs_rook | 0.50 | +0.000 | +0.001 | +0.001 | +0.005 | +0.005 | +0.004 | 0.690 | 0.568 | 0.530 |
| rr_3 | rook_vs_rook | 0.50 | -0.000 | -0.000 | -0.000 | +0.001 | +0.001 | +0.000 | 0.643 | 0.553 | 0.483 |
| blocked_wall | blocked_pawns | 0.50 | +0.000 | +0.000 | +0.000 | +0.000 | +0.000 | +0.000 | 0.870 | 0.935 | 0.963 |
| blocked_wall_black_to_move | blocked_pawns | 0.50 | +0.000 | +0.000 | +0.000 | +0.000 | +0.000 | +0.000 | 0.797 | 0.872 | 0.930 |
| blocked_wall_corner_kings | blocked_pawns | 0.50 | -0.000 | -0.000 | -0.000 | +0.000 | +0.000 | +0.000 | 0.788 | 0.866 | 0.911 |
| blocked_wall_low | blocked_pawns | 0.50 | +0.000 | +0.000 | +0.000 | +0.000 | +0.000 | +0.000 | 0.793 | 0.864 | 0.924 |
| kp_queenside_king_can_enter | kp_king_can_enter | 0.50 | +0.296 | +0.192 | +0.126 | +0.190 | +0.078 | +0.064 | 0.741 | 0.788 | 0.855 |
| kp_three_king_can_enter | kp_king_can_enter | 0.50 | +0.314 | +0.217 | +0.132 | +0.203 | +0.114 | +0.079 | 0.709 | 0.797 | 0.877 |
| sharp_najdorf_english_attack | sharp_opening | 0.55 | +0.170 | +0.169 | +0.212 | +0.290 | +0.231 | +0.154 | 0.017 | 0.017 | 0.021 |
| sharp_semislav_botvinnik | sharp_opening | 0.58 | +0.350 | +0.317 | +0.281 | +0.303 | +0.281 | +0.255 | 0.024 | 0.024 | 0.026 |
| sharp_kid_mar_del_plata | sharp_opening | 0.59 | +0.133 | +0.115 | +0.077 | +0.196 | +0.137 | +0.082 | 0.024 | 0.028 | 0.037 |
| sharp_dragon_yugoslav | sharp_opening | 0.64 | +0.362 | +0.311 | +0.208 | +0.207 | +0.165 | +0.144 | 0.021 | 0.021 | 0.025 |
| rbr_1_attacker_to_move | rb_vs_r | 0.50 | +0.003 | +0.002 | +0.002 | +0.026 | +0.008 | +0.000 | 0.471 | 0.319 | 0.255 |
| rbr_1_defender_to_move | rb_vs_r | 0.50 | -0.000 | -0.001 | -0.001 | +0.008 | +0.007 | +0.001 | 0.563 | 0.460 | 0.454 |
| rbr_2_attacker_to_move | rb_vs_r | 0.50 | +0.000 | +0.000 | -0.000 | +0.213 | +0.183 | +0.111 | 0.526 | 0.427 | 0.366 |
| rbr_2_defender_to_move | rb_vs_r | 0.51 | +0.003 | +0.002 | +0.002 | +0.153 | +0.119 | +0.058 | 0.546 | 0.424 | 0.390 |
| trap_blackburne_shilling | opening_trap | 0.93 | +0.509 | +0.424 | +0.382 | +0.017 | +0.023 | +0.014 | 0.024 | 0.025 | 0.033 |
| trap_englund | opening_trap | 0.95 | +0.145 | +0.155 | +0.187 | +0.044 | +0.044 | +0.044 | 0.026 | 0.025 | 0.029 |
| trap_elephant | opening_trap | 0.53 | +0.435 | +0.399 | +0.298 | +0.076 | +0.035 | +0.024 | 0.027 | 0.029 | 0.036 |
| trap_noahs_ark | opening_trap | 0.51 | +0.495 | +0.505 | +0.483 | +0.296 | +0.271 | +0.171 | 0.024 | 0.024 | 0.030 |
| trap_scholars_mate | opening_trap | 0.47 | +0.283 | +0.206 | +0.144 | +0.107 | +0.109 | +0.106 | 0.019 | 0.020 | 0.024 |
| trap_caro_kann_smothered | opening_trap | 0.52 | +0.234 | +0.174 | +0.083 | +0.384 | +0.368 | +0.281 | 0.023 | 0.025 | 0.033 |

| Check | Result | Detail |
| --- | --- | --- |
| 1100: rook vs rook, both T close to 0 | PASS | max +0.005 |
| 1100: blocked pawns, both T close to 0 | PASS | max +0.000 |
| 1100: sharp opening, T medium (above rook vs rook) | PASS | mean 0.251 vs 0.001 |
| 1100: R+B vs R, attacker T close to 0 | PASS | max +0.003 |
| 1100: R+B vs R, defender T above attacker T | PASS | defender 0.100 vs attacker 0.001 |
| 1100: trap, victim T > trapper T in every trap | **FAIL** | blackburne_shilling:0.51/0.02 englund:0.15/0.04 elephant:0.44/0.08 noahs_ark:0.50/0.30 scholars_mate:0.11/0.28 caro_kann_smothered:0.38/0.23 |
| 1100: D(rook vs rook) > D(sharp opening) | PASS | 0.672 > 0.021 |
| 1500: rook vs rook, both T close to 0 | PASS | max +0.005 |
| 1500: blocked pawns, both T close to 0 | PASS | max +0.000 |
| 1500: sharp opening, T medium (above rook vs rook) | PASS | mean 0.216 vs 0.001 |
| 1500: R+B vs R, attacker T close to 0 | PASS | max +0.002 |
| 1500: R+B vs R, defender T above attacker T | PASS | defender 0.079 vs attacker 0.001 |
| 1500: trap, victim T > trapper T in every trap | **FAIL** | blackburne_shilling:0.42/0.02 englund:0.16/0.04 elephant:0.40/0.03 noahs_ark:0.51/0.27 scholars_mate:0.11/0.21 caro_kann_smothered:0.37/0.17 |
| 1500: D(rook vs rook) > D(sharp opening) | PASS | 0.571 > 0.022 |
| 1900: rook vs rook, both T close to 0 | PASS | max +0.004 |
| 1900: blocked pawns, both T close to 0 | PASS | max +0.000 |
| 1900: sharp opening, T medium (above rook vs rook) | PASS | mean 0.177 vs 0.001 |
| 1900: R+B vs R, attacker T close to 0 | PASS | max +0.002 |
| 1900: R+B vs R, defender T above attacker T | PASS | defender 0.043 vs attacker 0.001 |
| 1900: trap, victim T > trapper T in every trap | **FAIL** | blackburne_shilling:0.38/0.01 englund:0.19/0.04 elephant:0.30/0.02 noahs_ark:0.48/0.17 scholars_mate:0.11/0.14 caro_kann_smothered:0.28/0.08 |
| 1900: D(rook vs rook) > D(sharp opening) | PASS | 0.524 > 0.027 |
| R+B vs R, defender T falls as rating rises | PASS | 0.100 > 0.079 > 0.043 |
| T falls as rating rises (mean over positions with T > near-zero) | PASS | 0.264 > 0.220 > 0.173 |
| T falls as rating rises (each position-side, lowest vs highest rating) | **FAIL** | 22/24 fall |
