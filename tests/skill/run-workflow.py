#!/usr/bin/env python3
"""Exercise the exact extracted binary. Any HOLD is a failed qualification."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import zipfile

ROOT=Path(__file__).resolve().parents[2]
FIXTURES=Path(__file__).resolve().parent/'fixtures'
MEMBERS={
 'flowspec/SKILL.md','flowspec/references/report-contract.md',
 'flowspec/references/examples.md','flowspec/LICENSE','flowspec/LICENSE-AGPL',
 'flowspec/bin/flowspec','flowspec/package.json','flowspec/SHA256SUMS',
}


NOTICE_HASHES = {'THIRD-PARTY-LICENSES/aho-corasick-1.1.4/COPYING': '01c266bced4a434da0051174d6bee16a4c82cf634e2679b6155d40d75012390f', 'THIRD-PARTY-LICENSES/aho-corasick-1.1.4/LICENSE-MIT': '0f96a83840e146e43c0ec96a22ec1f392e0680e6c1226e6f3ba87e0740af850f', 'THIRD-PARTY-LICENSES/aho-corasick-1.1.4/UNLICENSE': '7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c', 'THIRD-PARTY-LICENSES/anstream-0.6.21/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/anstream-0.6.21/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/anstyle-1.0.13/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/anstyle-1.0.13/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/anstyle-parse-0.2.7/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/anstyle-parse-0.2.7/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/anstyle-query-1.1.5/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/anstyle-query-1.1.5/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/anyhow-1.0.103/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/anyhow-1.0.103/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/autocfg-1.5.0/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/autocfg-1.5.0/LICENSE-MIT': '27995d58ad5c1145c1a8cd86244ce844886958a35eb2b78c6b772748669999ac', 'THIRD-PARTY-LICENSES/bincode-2.0.1/LICENSE.md': '90d7e062634054e6866d3c81e6a2b3058a840e6af733e98e80bdfe1a7dec6912', 'THIRD-PARTY-LICENSES/bincode_derive-2.0.1/LICENSE.md': '90d7e062634054e6866d3c81e6a2b3058a840e6af733e98e80bdfe1a7dec6912', 'THIRD-PARTY-LICENSES/block-buffer-0.10.4/LICENSE-APACHE': 'a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5', 'THIRD-PARTY-LICENSES/block-buffer-0.10.4/LICENSE-MIT': 'd5c22aa3118d240e877ad41c5d9fa232f9c77d757d4aac0c2f943afc0a95e0ef', 'THIRD-PARTY-LICENSES/bstr-1.12.1/COPYING': '68653aaa727a2bfa31b7a751e31701ce33c49d695c12dd291a07d1c54da4c14b', 'THIRD-PARTY-LICENSES/bstr-1.12.1/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/bstr-1.12.1/LICENSE-MIT': '6b7374c39a57e57fc2c38eb529c4c88340152b10f51dd5ae2d819dfa67f61715', 'THIRD-PARTY-LICENSES/cc-1.2.56/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/cc-1.2.56/LICENSE-MIT': '378f5840b258e2779c39418f3f2d7b2ba96f1c7917dd6be0713f88305dbda397', 'THIRD-PARTY-LICENSES/cfg-if-1.0.4/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/cfg-if-1.0.4/LICENSE-MIT': '378f5840b258e2779c39418f3f2d7b2ba96f1c7917dd6be0713f88305dbda397', 'THIRD-PARTY-LICENSES/chrono-0.4.44/LICENSE.txt': '946c9835d8034d24404f8cfec5f4654cee5dad17e944afc3d06d742cf2882831', 'THIRD-PARTY-LICENSES/clap-4.5.60/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/clap-4.5.60/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/clap_builder-4.5.60/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/clap_builder-4.5.60/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/clap_derive-4.5.55/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/clap_derive-4.5.55/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/clap_lex-1.0.0/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/clap_lex-1.0.0/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/colorchoice-1.0.4/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/colorchoice-1.0.4/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/cpufeatures-0.2.17/LICENSE-APACHE': 'a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5', 'THIRD-PARTY-LICENSES/cpufeatures-0.2.17/LICENSE-MIT': 'ae9baa7beea910273c2f384c2a6b721fb7bd02bda3436074a1072e4ee689f985', 'THIRD-PARTY-LICENSES/crossbeam-deque-0.8.6/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/crossbeam-deque-0.8.6/LICENSE-MIT': '5734ed989dfca1f625b40281ee9f4530f91b2411ec01cb748223e7eb87e201ab', 'THIRD-PARTY-LICENSES/crossbeam-epoch-0.9.20/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/crossbeam-epoch-0.9.20/LICENSE-MIT': '5734ed989dfca1f625b40281ee9f4530f91b2411ec01cb748223e7eb87e201ab', 'THIRD-PARTY-LICENSES/crossbeam-utils-0.8.21/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/crossbeam-utils-0.8.21/LICENSE-MIT': '5734ed989dfca1f625b40281ee9f4530f91b2411ec01cb748223e7eb87e201ab', 'THIRD-PARTY-LICENSES/crypto-common-0.1.7/LICENSE-APACHE': 'a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5', 'THIRD-PARTY-LICENSES/crypto-common-0.1.7/LICENSE-MIT': '3521672491a3479422d5fe1aca6645dd2984090f85da6e5205abfb18fb7a6897', 'THIRD-PARTY-LICENSES/digest-0.10.7/LICENSE-APACHE': 'a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5', 'THIRD-PARTY-LICENSES/digest-0.10.7/LICENSE-MIT': '9e0dfd2dd4173a530e238cb6adb37aa78c34c6bc7444e0e10c1ab5d8881f63ba', 'THIRD-PARTY-LICENSES/equivalent-1.0.2/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/equivalent-1.0.2/LICENSE-MIT': '7365cc8878a1d7ce155a58c4ca09c3d7a6be413efa5334a80ea842912b669349', 'THIRD-PARTY-LICENSES/find-msvc-tools-0.1.9/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/find-msvc-tools-0.1.9/LICENSE-MIT': '378f5840b258e2779c39418f3f2d7b2ba96f1c7917dd6be0713f88305dbda397', 'THIRD-PARTY-LICENSES/generic-array-0.14.7/LICENSE': 'c09aae9d3c77b531f56351a9947bc7446511d6b025b3255312d3e3442a9a7583', 'THIRD-PARTY-LICENSES/glob-0.3.3/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/glob-0.3.3/LICENSE-MIT': '6485b8ed310d3f0340bf1ad1f47645069ce4069dcc6bb46c7d5c6faf41de1fdb', 'THIRD-PARTY-LICENSES/globset-0.4.18/COPYING': '01c266bced4a434da0051174d6bee16a4c82cf634e2679b6155d40d75012390f', 'THIRD-PARTY-LICENSES/globset-0.4.18/LICENSE-MIT': '0f96a83840e146e43c0ec96a22ec1f392e0680e6c1226e6f3ba87e0740af850f', 'THIRD-PARTY-LICENSES/globset-0.4.18/UNLICENSE': '7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c', 'THIRD-PARTY-LICENSES/hashbrown-0.16.1/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/hashbrown-0.16.1/LICENSE-MIT': 'ff8f68cb076caf8cefe7a6430d4ac086ce6af2ca8ce2c4e5a2004d4552ef52a2', 'THIRD-PARTY-LICENSES/heck-0.5.0/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/heck-0.5.0/LICENSE-MIT': '7b63ecd5f1902af1b63729947373683c32745c16a10e8e6292e2e2dcd7e90ae0', 'THIRD-PARTY-LICENSES/iana-time-zone-0.1.65/LICENSE-APACHE': '696759d65dfe558ff7d9f031c76db19ec5c0767470fb67c4e8d990820d1e99c9', 'THIRD-PARTY-LICENSES/iana-time-zone-0.1.65/LICENSE-MIT': 'da28ccc6b158fc2d8cccc74e99794b1cff1d29bd7bbeb019442fcf0c04c6cad9', 'THIRD-PARTY-LICENSES/ignore-0.4.25/COPYING': '01c266bced4a434da0051174d6bee16a4c82cf634e2679b6155d40d75012390f', 'THIRD-PARTY-LICENSES/ignore-0.4.25/LICENSE-MIT': '0f96a83840e146e43c0ec96a22ec1f392e0680e6c1226e6f3ba87e0740af850f', 'THIRD-PARTY-LICENSES/ignore-0.4.25/UNLICENSE': '7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c', 'THIRD-PARTY-LICENSES/index.json': 'cd50879298d1ab28a600983291d55c11b09d96f7f35216b5d94875222ecbf69f', 'THIRD-PARTY-LICENSES/indexmap-2.13.0/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/indexmap-2.13.0/LICENSE-MIT': 'ecc269ef87fd38a1d98e30bfac9ba964a9dbd9315c3770fed98d4d7cb5882055', 'THIRD-PARTY-LICENSES/is_terminal_polyfill-1.70.2/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/is_terminal_polyfill-1.70.2/LICENSE-MIT': '6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6', 'THIRD-PARTY-LICENSES/itoa-1.0.17/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/itoa-1.0.17/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/lazy_static-1.5.0/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/lazy_static-1.5.0/LICENSE-MIT': '0621878e61f0d0fda054bcbe02df75192c28bde1ecc8289cbd86aeba2dd72720', 'THIRD-PARTY-LICENSES/log-0.4.29/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/log-0.4.29/LICENSE-MIT': '6485b8ed310d3f0340bf1ad1f47645069ce4069dcc6bb46c7d5c6faf41de1fdb', 'THIRD-PARTY-LICENSES/matchers-0.2.0/LICENSE': 'a47129d738752a6ae52247fea645b42cb320d19e9327f1eb2d1a7f99d9455ad3', 'THIRD-PARTY-LICENSES/memchr-2.8.0/COPYING': '01c266bced4a434da0051174d6bee16a4c82cf634e2679b6155d40d75012390f', 'THIRD-PARTY-LICENSES/memchr-2.8.0/LICENSE-MIT': '0f96a83840e146e43c0ec96a22ec1f392e0680e6c1226e6f3ba87e0740af850f', 'THIRD-PARTY-LICENSES/memchr-2.8.0/UNLICENSE': '7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c', 'THIRD-PARTY-LICENSES/nu-ansi-term-0.50.3/LICENSE': '284465860407420254e39be4e1bdcaaf2e7f2d18e06f9bab75bc75341a5d8501', 'THIRD-PARTY-LICENSES/num-traits-0.2.19/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/num-traits-0.2.19/LICENSE-MIT': '6485b8ed310d3f0340bf1ad1f47645069ce4069dcc6bb46c7d5c6faf41de1fdb', 'THIRD-PARTY-LICENSES/once_cell-1.21.3/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/once_cell-1.21.3/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/pin-project-lite-0.2.17/LICENSE-APACHE': '0d542e0c8804e39aa7f37eb00da5a762149dc682d7829451287e11b938e94594', 'THIRD-PARTY-LICENSES/pin-project-lite-0.2.17/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/proc-macro2-1.0.106/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/proc-macro2-1.0.106/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/quote-1.0.45/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/quote-1.0.45/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/regex-1.12.3/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/regex-1.12.3/LICENSE-MIT': '6485b8ed310d3f0340bf1ad1f47645069ce4069dcc6bb46c7d5c6faf41de1fdb', 'THIRD-PARTY-LICENSES/regex-automata-0.4.14/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/regex-automata-0.4.14/LICENSE-MIT': '6485b8ed310d3f0340bf1ad1f47645069ce4069dcc6bb46c7d5c6faf41de1fdb', 'THIRD-PARTY-LICENSES/regex-syntax-0.8.10/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/regex-syntax-0.8.10/LICENSE-MIT': '6485b8ed310d3f0340bf1ad1f47645069ce4069dcc6bb46c7d5c6faf41de1fdb', 'THIRD-PARTY-LICENSES/regex-syntax-0.8.10/src/unicode_tables/LICENSE-UNICODE': '74db5baf44a41b1000312c673544b3374e4198af5605c7f9080a402cec42cfa3', 'THIRD-PARTY-LICENSES/ryu-1.0.23/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/ryu-1.0.23/LICENSE-BOOST': 'c9bff75738922193e67fa726fa225535870d2aa1059f91452c411736284ad566', 'THIRD-PARTY-LICENSES/same-file-1.0.6/COPYING': '01c266bced4a434da0051174d6bee16a4c82cf634e2679b6155d40d75012390f', 'THIRD-PARTY-LICENSES/same-file-1.0.6/LICENSE-MIT': 'cb3c929a05e6cbc9de9ab06a4c57eeb60ca8c724bef6c138c87d3a577e27aa14', 'THIRD-PARTY-LICENSES/same-file-1.0.6/UNLICENSE': '7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c', 'THIRD-PARTY-LICENSES/serde-1.0.228/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/serde-1.0.228/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/serde_core-1.0.228/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/serde_core-1.0.228/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/serde_derive-1.0.228/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/serde_derive-1.0.228/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/serde_json-1.0.149/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/serde_json-1.0.149/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/serde_yaml-0.9.34+deprecated/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/serde_yaml-0.9.34+deprecated/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/sha2-0.10.9/LICENSE-APACHE': 'a9040321c3712d8fd0b09cf52b17445de04a23a10165049ae187cd39e5c86be5', 'THIRD-PARTY-LICENSES/sha2-0.10.9/LICENSE-MIT': 'b4eb00df6e2a4d22518fcaa6a2b4646f249b3a3c9814509b22bd2091f1392ff1', 'THIRD-PARTY-LICENSES/sharded-slab-0.1.7/LICENSE': 'eafbfa606bc005ed7fd2f623d65af17ffe4ef7c017221ac14405bf4140771ea9', 'THIRD-PARTY-LICENSES/shlex-1.3.0/LICENSE-APACHE': '553fffcd9b1cb158bc3e9edc35da85ca5c3b3d7d2e61c883ebcfa8a65814b583', 'THIRD-PARTY-LICENSES/shlex-1.3.0/LICENSE-MIT': '4455bf75a91154108304cb283e0fea9948c14f13e20d60887cf2552449dea3b1', 'THIRD-PARTY-LICENSES/slotmap-1.1.1/LICENSE': '6cec81441b2ab2b5090954c1fb5d0bcca81b86487b5849061fa8952d20ef67d4', 'THIRD-PARTY-LICENSES/smallvec-1.15.1/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/smallvec-1.15.1/LICENSE-MIT': '0b28172679e0009b655da42797c03fd163a3379d5cfa67ba1f1655e974a2a1a9', 'THIRD-PARTY-LICENSES/streaming-iterator-0.1.9/LICENSE-APACHE': 'c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08', 'THIRD-PARTY-LICENSES/streaming-iterator-0.1.9/LICENSE-MIT': '34cb64162194d52a8412e96d8693ebf43a67d019c6b5bd76da3bb377ba7507aa', 'THIRD-PARTY-LICENSES/strsim-0.11.1/LICENSE': '1e697ce8d21401fbf1bddd9b5c3fd4c4c79ae1e3bdf51f81761c85e11d5a89cd', 'THIRD-PARTY-LICENSES/syn-2.0.117/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/syn-2.0.117/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/thiserror-2.0.18/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/thiserror-2.0.18/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/thiserror-impl-2.0.18/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/thiserror-impl-2.0.18/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/thread_local-1.1.9/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/thread_local-1.1.9/LICENSE-MIT': 'c9a75f18b9ab2927829a208fc6aa2cf4e63b8420887ba29cdb265d6619ae82d5', 'THIRD-PARTY-LICENSES/tracing-0.1.44/LICENSE': '898b1ae9821e98daf8964c8d6c7f61641f5f5aa78ad500020771c0939ee0dea1', 'THIRD-PARTY-LICENSES/tracing-attributes-0.1.31/LICENSE': '898b1ae9821e98daf8964c8d6c7f61641f5f5aa78ad500020771c0939ee0dea1', 'THIRD-PARTY-LICENSES/tracing-core-0.1.36/LICENSE': '898b1ae9821e98daf8964c8d6c7f61641f5f5aa78ad500020771c0939ee0dea1', 'THIRD-PARTY-LICENSES/tracing-core-0.1.36/src/spin/LICENSE': '58545fed1565e42d687aecec6897d35c6d37ccb71479a137c0deb2203e125c79', 'THIRD-PARTY-LICENSES/tracing-log-0.2.0/LICENSE': '898b1ae9821e98daf8964c8d6c7f61641f5f5aa78ad500020771c0939ee0dea1', 'THIRD-PARTY-LICENSES/tracing-subscriber-0.3.22/LICENSE': '898b1ae9821e98daf8964c8d6c7f61641f5f5aa78ad500020771c0939ee0dea1', 'THIRD-PARTY-LICENSES/tree-sitter-0.25.10/LICENSE-UPSTREAM': '5f9cf9fb6acb1972b35ae29119ce563bb60ec097656bc4b69b9bac2d04c7a147', 'THIRD-PARTY-LICENSES/tree-sitter-0.25.10/src/unicode/LICENSE': '6a18c5fac70d7860b57f5b72b4e2c9a1ba6b3d2741eef7ff9767c5379364f10d', 'THIRD-PARTY-LICENSES/tree-sitter-javascript-0.25.0/LICENSE': '2e0110e07abef7c2548b26ec9d6969775617ca539a0dc8dbeeb14d6452c711d1', 'THIRD-PARTY-LICENSES/tree-sitter-language-0.1.7/LICENSE': 'c5cfb43042b6b72045f4ba997834d0a7786d2793d91680868b5815b39f14fc78', 'THIRD-PARTY-LICENSES/tree-sitter-python-0.25.0/LICENSE': 'd724405ce238a22c0d35769c5a36b386ad5958192efe8bbb304fb2896254575f', 'THIRD-PARTY-LICENSES/tree-sitter-rust-0.24.0/LICENSE-UPSTREAM': '31d5b6f4243d5c7c6e1c4ebbbb9f6407bd1457a08bcc4f706521710341acba36', 'THIRD-PARTY-LICENSES/tree-sitter-typescript-0.23.2/LICENSE-UPSTREAM': '49bf33cf78ef5897e4e161ce1517df7de1ae5042a65b6bcfd44401e0fc606559', 'THIRD-PARTY-LICENSES/typenum-1.19.0/LICENSE': 'db11fec9946737df39ca3898d9cd8c10ec6f6c3a884a6802b0ad0b81b4e8f23a', 'THIRD-PARTY-LICENSES/typenum-1.19.0/LICENSE-APACHE': '516b24e051bf5630880ebbd55c40a25ce9552ebaf8970a53e8976eb70e522406', 'THIRD-PARTY-LICENSES/typenum-1.19.0/LICENSE-MIT': 'a825bd853ab71619a4923d7b4311221427848070ff44d990da39b0b274c1683f', 'THIRD-PARTY-LICENSES/unicode-ident-1.0.24/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/unicode-ident-1.0.24/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/unicode-ident-1.0.24/LICENSE-UNICODE': 'f7db81051789b729fea528a63ec4c938fdcb93d9d61d97dc8cc2e9df6d47f2a1', 'THIRD-PARTY-LICENSES/unsafe-libyaml-0.2.11/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-LICENSES/unty-0.0.4/LICENSE-APACHE': 'fa84f04c495f2533ba036606acc8644b50077752f17bc2e943235459dda1c12c', 'THIRD-PARTY-LICENSES/unty-0.0.4/LICENSE-MIT': 'ac1e6e437cd571f6b450abd33dc055cf26dfb0fe2a72952c3ef6ae3844549c12', 'THIRD-PARTY-LICENSES/utf8parse-0.2.2/LICENSE-APACHE': '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a', 'THIRD-PARTY-LICENSES/utf8parse-0.2.2/LICENSE-MIT': 'e4c9b06fa850cb9b540a5e400e9f6394cf15efcf4098144de477d1d3dae10150', 'THIRD-PARTY-LICENSES/version_check-0.9.5/LICENSE-APACHE': 'a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2', 'THIRD-PARTY-LICENSES/version_check-0.9.5/LICENSE-MIT': 'b7e650f3fce5c53249d1cdc608b54df156a97edd636cf9d23498d0cfe7aec63e', 'THIRD-PARTY-LICENSES/virtue-0.0.18/LICENSE.md': 'ddcbb6914b62d5bebc3cb58ebe8d2738ffa9a48555469bbcbe65159979b878cf', 'THIRD-PARTY-LICENSES/walkdir-2.5.0/COPYING': '01c266bced4a434da0051174d6bee16a4c82cf634e2679b6155d40d75012390f', 'THIRD-PARTY-LICENSES/walkdir-2.5.0/LICENSE-MIT': '0f96a83840e146e43c0ec96a22ec1f392e0680e6c1226e6f3ba87e0740af850f', 'THIRD-PARTY-LICENSES/walkdir-2.5.0/UNLICENSE': '7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c', 'THIRD-PARTY-LICENSES/zmij-1.0.21/LICENSE-MIT': '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3', 'THIRD-PARTY-NOTICES': 'da7bd3905dc191e83f721cdeee883a6ee095031d36c17036d0b2ce31ce7a806d'}
MEMBERS |= {'flowspec/'+name for name in NOTICE_HASHES}


def digest(data):return hashlib.sha256(data).hexdigest()


def extract(archive,dest):
    with zipfile.ZipFile(archive) as z:
        if len(z.infolist())!=len(MEMBERS) or set(z.namelist())!=MEMBERS:
            raise ValueError('archive allowlist mismatch or duplicate members')
        if sum(i.file_size for i in z.infolist())>260*1024*1024:
            raise ValueError('oversize archive')
        for info in z.infolist():
            mode=info.external_attr>>16
            if stat.S_ISLNK(mode) or not stat.S_ISREG(mode):
                raise ValueError('unsafe archive member type')
        rows=z.read('flowspec/SHA256SUMS').decode().splitlines()
        sums={}
        for row in rows:
            sha,name=row.split('  ')
            if name in sums or 'flowspec/'+name not in MEMBERS:
                raise ValueError('unsafe or duplicate checksum member')
            sums[name]=sha
        if set(sums)!={m.removeprefix('flowspec/') for m in MEMBERS if m!='flowspec/SHA256SUMS'}:
            raise ValueError('checksum coverage mismatch')
        for name,sha in sums.items():
            if digest(z.read('flowspec/'+name))!=sha:raise ValueError('archive checksum mismatch: '+name)
        manifest=json.loads(z.read('flowspec/package.json'))
        if digest(z.read('flowspec/bin/flowspec'))!=manifest['binary_sha256']:
            raise ValueError('binary manifest mismatch')
        reviewed=os.environ.get('FLOWSPEC_SKILL_BINARY')
        if not reviewed:
            raise ValueError('exact reviewed FLOWSPEC_SKILL_BINARY is required')
        reviewed_path=Path(reviewed)
        if not reviewed_path.is_file() or any(p.is_symlink() for p in (reviewed_path,*reviewed_path.parents)):
            raise ValueError('invalid reviewed binary input')
        if digest(reviewed_path.read_bytes())!=manifest['binary_sha256']:
            raise ValueError('unknown binary differs from reviewed candidate')
        if manifest['schema']!='flowspec.skill-package.v1':raise ValueError('unknown package schema')
        for name, expected_sha in NOTICE_HASHES.items():
            if digest(z.read('flowspec/'+name)) != expected_sha:
                raise ValueError('canonical source notice mismatch: '+name)
        if manifest.get('third_party') != {'index_sha256':NOTICE_HASHES['THIRD-PARTY-LICENSES/index.json'], 'members':NOTICE_HASHES}:
            raise ValueError('canonical source notice declaration mismatch')
        payloads = {m.removeprefix('flowspec/'):digest(z.read(m)) for m in MEMBERS if m not in {'flowspec/package.json','flowspec/SHA256SUMS'}}
        if manifest.get('members') != payloads:
            raise ValueError('manifest member coverage mismatch')
        if manifest.get('release_qualified') is not False:
            raise ValueError('development archive cannot assert release qualification')
        head=subprocess.check_output(['git','-C',str(ROOT),'rev-parse','HEAD'],text=True).strip()
        tree=subprocess.check_output(['git','-C',str(ROOT),'rev-parse','HEAD^{tree}'],text=True).strip()
        producer=manifest.get('binary_producer',{})
        if (manifest.get('source_sha')!=head or manifest.get('source_tree')!=tree
                or producer.get('build_sha')!=head or producer.get('source_tree_sha')!=tree
                or producer.get('generated_by')!='analyzer' or producer.get('report_abi')!='v1'):
            raise ValueError('package producer/source differs from current source')
        for info in z.infolist():
            path=dest/info.filename
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes(z.read(info.filename))
            path.chmod(0o755 if info.filename.endswith('/bin/flowspec') else 0o644)
    return dest/'flowspec/bin/flowspec',manifest


IDENTITY = ['schema_version','producer_contract','producer','execution','outcome','findings_present','gate','findings','scope','provenance','inventory','losses','pattern_state','analysis_graph']
PATTERNS = {'isolated_cluster','data_dead_end','partial_wiring','orphaned_impl','duplication','contract_mismatch','circular_dependency','layer_violation','incomplete_migration','asymmetric_handling','stale_reference','phantom_dependency','missing_reexport'}
def domain_hash(domain,value):
    payload=json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
    return 'sha256:'+hashlib.sha256(('flowspec/'+domain+'/v1\0').encode()+payload).hexdigest()

def source_expectation(source):
    # AST and literal source establish identities before producer output exists.
    import ast
    tree=ast.parse(source);fn=tree.body[0]
    assert len(tree.body)==1 and isinstance(fn,ast.FunctionDef) and fn.name=='_unused'
    assert len(fn.body)==1 and isinstance(fn.body[0],ast.Return) and isinstance(fn.body[0].value,ast.Constant)
    assert not any(isinstance(n,ast.Call) for n in ast.walk(tree))
    region=dict(path_key='6d61696e2e7079',start_byte=0,end_byte=27,start_line=1,end_line=2,start_column=1,end_column=13)
    base=dict(path_key='6d61696e2e7079',language='python',qualified_name='main.py::_unused',construct_kind='function')
    binding=domain_hash('binding',base);subject=dict(base,binding_id=binding);expected={}
    for pattern in ['data_dead_end','isolated_cluster']:
        parts=dict(produced_binding_id=binding,produced_kind='function',unconsumed_relation='unconsumed_private_symbol')
        evidence=[dict(kind='graph_inbound_consumption',observation='0 callers in 1 analyzed files',range=None)]
        id_subject=base
        if pattern=='isolated_cluster':
            parts.update(unconsumed_relation='closed_private_executable_component',component_members=[binding]);id_subject=subject
            evidence=[dict(kind='executable_component_closure',consumer_closure='complete',member_binding_ids=[binding],external_inbound=0,internal_executable_edges=0,lexical_containment_is_consumption=False)]
        cause=dict(key_version=1,parts=parts)
        sid=domain_hash('finding.semantic',dict(pattern_id=pattern,subject=id_subject,cause=cause))
        obs=dict(semantic_id=sid,range=region,evidence=evidence)
        if pattern=='data_dead_end':obs['source_sha256']='sha256:'+digest(source.encode())
        else:obs['source_observation']=expected['data_dead_end']['observation_id']
        expected[pattern]=dict(pattern_id=pattern,identity_status='stable',subject=subject,range=region,cause=cause,semantic_id=sid,observation_id=domain_hash('finding.observation',obs),evidence=evidence,severity='warning',confidence='high')
    assert expected['data_dead_end']['semantic_id']=='sha256:fe11a5179a94c2a3c0b20f305fd4cf913c254d1d514beb4f4b93a3e3527e49af'
    return expected

def private_contract(authority,expected):
    rows=authority.get('findings',{}).get('all',[])
    if len(rows)!=2 or {f.get('pattern_id') for f in rows}!=set(expected):return False
    if len({f.get('semantic_id') for f in rows})!=2 or len({f.get('observation_id') for f in rows})!=2:return False
    if authority.get('execution')!='complete' or authority.get('outcome')!='findings' or authority.get('losses')!=[]:return False
    if authority.get('gate')!={'qualifying_count':2,'verdict':'fail'}:return False
    if authority['findings']['total']!=2 or set(authority['findings']['by_pattern'])!=PATTERNS:return False
    if any(authority['findings']['by_pattern'][p]!=(1 if p in expected else 0) for p in PATTERNS):return False
    if set(authority)!=set(IDENTITY+['report_identity','display','continuation']):return False
    if authority['report_identity']!=domain_hash('report',{k:authority[k] for k in IDENTITY}):return False
    states=authority['pattern_state']
    if len(states)!=13 or {s.get('pattern_id') for s in states}!=PATTERNS:return False
    policy_absent={'partial_wiring','layer_violation','incomplete_migration','asymmetric_handling','missing_reexport'}
    capabilities={
      'isolated_cluster':['calls','resolution','symbols'],
      'data_dead_end':['calls','consumer_closure','exports','registration','symbols','value_references'],
      'partial_wiring':['calls','control_flow','symbols'],
      'orphaned_impl':['calls','implementations','resolution','symbols','types'],
      'duplication':['binding_structure','symbols'],
      'contract_mismatch':['calls','control_flow','symbols','types'],
      'circular_dependency':['dependency_edges','imports'],
      'layer_violation':['dependency_edges','imports'],
      'incomplete_migration':['imports','resolution'],
      'asymmetric_handling':['calls','control_flow','symbols'],
      'stale_reference':['imports','resolution'],
      'phantom_dependency':['exports','imports','value_references'],
      'missing_reexport':['exports','imports','value_references'],
    }
    for state in states:
        pattern=state['pattern_id']; absent=pattern in policy_absent
        if state!={'pattern_id':pattern,'cells':[{'affected_paths':[],'language':'python',
                    'reason_codes':['policy_absent'] if absent else [],
                    'required_capabilities':capabilities[pattern],
                    'state':'not_applicable' if absent else 'complete'}]}:return False
    # No calls, annotations, parameters, module boundaries or types exist in the
    # source fixture. Verify the entire current encoded Graph, including table
    # bindings and exact source ranges; a consistent report hash is insufficient.
    region=expected['data_dead_end']['range']; subject=expected['data_dead_end']['subject']
    graph_expected={'encoding':'flowspec.graph.tables.v2',
       'tables':{'kinds':['function'],'languages':['python'],'names':[[True,subject['qualified_name'].split('::')[-1]]],'paths':['main.py']},
       'entities':[[0,0,0,0,region['start_byte'],region['end_byte'],region['start_line'],region['start_column'],region['end_line'],region['end_column']]],
       'boundaries':[],'dependency_graph':[],'flows':[],'type_flows':[],
       'compatibility_losses':'/authority/losses','compatibility_pattern_state':'/authority/pattern_state'}
    if authority.get('analysis_graph')!=graph_expected:return False
    return all(all(f.get(k)==v for k,v in expected[f['pattern_id']].items()) for f in rows)

def private_legacy_contract(report,expected):
    authority=report.get('authority',{})
    return (private_contract(authority,expected)
        and report.get('entities')==[{'annotations':[],'called_by':[],'calls':[],
            'id':'main.py::_unused','kind':'fn','loc':'main.py:1','sig':'()','vis':'priv'}]
        and all(report.get(name)==[] for name in ['boundaries','dependency_graph','flows','type_flows'])
        and report.get('pattern_state')==[] # wire alias restored by typed Manifest reader
        and report.get('losses')==[])

def run_workflow(archive,out):
    archive,out=Path(archive).resolve(),Path(out).resolve()
    out.mkdir(parents=True,exist_ok=True)
    expected=source_expectation((FIXTURES/'findings.py').read_text())
    result={'schema':'flowspec.skill-workflow.v1','archive':str(archive),'archive_sha256':digest(archive.read_bytes()),'import_path':str(Path(__file__).resolve()),'earned':[],'holds':[],'commands':[], 'release_qualified':False, 'fixture_digests':{p.name:digest(p.read_bytes()) for p in sorted(FIXTURES.glob('*.py'))}}
    def record(name,passed,observed,expected):
        if passed:result['earned'].append(name)
        else:result['holds'].append({'operation':name,'status':'HOLD','expected':expected,'observed':observed})
    with tempfile.TemporaryDirectory(prefix='flowspec-skill-workflow-') as t:
        sandbox=Path(t);binary,manifest=extract(archive,sandbox)
        result['package_manifest']=manifest
        result['binary_sha256']=digest(binary.read_bytes())
        home=sandbox/'empty-home';home.mkdir()
        env={'HOME':str(home),'PATH':os.defpath,'LC_ALL':'C','TZ':'UTC'}
        def invoke(label,args):
            r=subprocess.run([str(binary),*map(str,args)],cwd=sandbox,env=env,capture_output=True,timeout=30)
            stdout=out/(label+'.stdout');stderr=out/(label+'.stderr')
            stdout.write_bytes(r.stdout);stderr.write_bytes(r.stderr)
            result['commands'].append({'label':label,'argv':[str(binary),*map(str,args)],'exit':r.returncode,'stdout':str(stdout),'stderr':str(stderr),'stdout_sha256':digest(r.stdout),'stderr_sha256':digest(r.stderr)})
            try:value=json.loads(r.stdout)
            except (ValueError,UnicodeError):value=None
            if value is not None and args[0] in ('analyze','diagnose'):
                decoded=subprocess.run([str(binary),'expand-report','--quiet'],input=r.stdout,cwd=sandbox,env=env,capture_output=True,timeout=30)
                (out/(label+'.decoded.stdout')).write_bytes(decoded.stdout)
                (out/(label+'.decoded.stderr')).write_bytes(decoded.stderr)
                result['commands'].append({'label':label+'-expand','argv':[str(binary),'expand-report','--quiet'],'exit':decoded.returncode,'stdin':str(stdout),'stdout':str(out/(label+'.decoded.stdout')),'stderr':str(out/(label+'.decoded.stderr')),'stdout_sha256':digest(decoded.stdout),'stderr_sha256':digest(decoded.stderr)})
                if decoded.returncode!=0:raise ValueError('public report expansion failed: '+label)
                value=json.loads(decoded.stdout)
            return r.returncode,value,r.stdout.decode(errors='replace')
        version=invoke('version',['--version'])
        result['version']=version[2].strip()
        scopes={}
        for kind in ['clean','findings','incomplete','trace']:
            scope=sandbox/kind;scope.mkdir();(scope/'main.py').write_bytes((FIXTURES/(kind+'.py')).read_bytes());scopes[kind]=scope
        report_args=['--format','json','--quiet','--gate-severity','info','--gate-confidence','low']
        def analyze(kind,label=None,extra=()):
            return invoke(label or kind,['analyze',scopes[kind],*report_args,*extra])
        clean_code,clean,_=analyze('clean')
        a=clean.get('authority',{}) if isinstance(clean,dict) else {}
        record('clean_exit_0',clean_code==0 and a.get('execution')=='complete' and a.get('outcome')=='clean' and a.get('findings',{}).get('total')==0,{'exit':clean_code,'execution':a.get('execution'),'outcome':a.get('outcome')},'exit 0, complete clean report, zero full findings')
        result['observed_producer']=a.get('producer')
        if a.get('provenance',{}).get('registry',{}).get('digest')!=manifest.get('registry_digest'):
            raise ValueError('package registry provenance mismatch')
        # Replay the actual declared package preflight threshold, rather than
        # treating this workflow's explicitly different info/low gate as equal.
        preflight_scope=sandbox/'package-preflight';preflight_scope.mkdir()
        (preflight_scope/'main.py').write_bytes(b'')
        pc,pv,_=invoke('package-preflight',['analyze',preflight_scope,'--format','json','--quiet'])
        pa=pv.get('authority',{}) if isinstance(pv,dict) else {}
        if (pc!=0 or pa.get('execution')!='complete' or pa.get('outcome')!='clean'
                or pa.get('provenance',{}).get('config',{}).get('effective_digest')!=manifest.get('preflight_config_digest')):
            raise ValueError('package config preflight provenance mismatch')
        if a.get('producer')!=manifest['binary_producer']:raise ValueError('workflow producer differs from package-bound binary')
        error_code,error,_=invoke('error',['analyze',sandbox/'missing',*report_args])
        record('error_exit_1',error_code==1 and error is None,{'exit':error_code,'report':error},'exit 1; no contract-valid report for missing path')
        f_code,findings,_=analyze('findings')
        fa=findings.get('authority',{}) if isinstance(findings,dict) else {}
        found=fa.get('findings',{}).get('all',[])
        record('findings_exit_2',f_code==2 and fa.get('execution')=='complete' and fa.get('outcome')=='findings' and private_legacy_contract(findings,expected),{'exit':f_code,'execution':fa.get('execution'),'outcome':fa.get('outcome'),'count':len(found)},'exit 2 with exact source-derived private DDE and isolated-component findings')
        i_code,incomplete,_=analyze('incomplete')
        ia=incomplete.get('authority',{}) if isinstance(incomplete,dict) else {}
        record('incomplete_exit_3',i_code==3 and ia.get('execution')=='incomplete' and ia.get('outcome')=='incomplete' and bool(ia.get('losses')),{'exit':i_code,'execution':ia.get('execution'),'outcome':ia.get('outcome'),'losses':ia.get('losses')},'exit 3 with explicit selected parse loss')
        d_code,diagnosis,_=invoke('diagnose',['diagnose',scopes['findings'],*report_args])
        da=diagnosis.get('authority',{}) if isinstance(diagnosis,dict) else {}
        exact=da.get('findings',{}).get('all',[])
        expected_range={'start_byte':0,'end_byte':27,'start_line':1,'end_line':2,'start_column':1,'end_column':13,'path_key':'6d61696e2e7079'}
        expected_id='sha256:fe11a5179a94c2a3c0b20f305fd4cf913c254d1d514beb4f4b93a3e3527e49af'
        record('diagnose_exact',d_code==2 and private_legacy_contract(diagnosis,expected) and exact==found and all(da[k]==fa[k] for k in IDENTITY+['report_identity']),{'exit':d_code,'findings':exact},'exact two distinct source-derived causes, ranges, stable/observation IDs, binding and evidence; no extra finding')
        tr_code,trace,_=invoke('trace',['trace',scopes['trace'],'--symbol','main.py::main','--direction','forward','--format','json','--quiet'])
        paths=[{'entry':p.get('entry'),'exit':p.get('exit'),'steps':[(s.get('entity'),s.get('action')) for s in p.get('steps',[])]} for p in trace] if isinstance(trace,list) else None
        expected_paths=[{'entry':'main.py::main','exit':'main.py::leaf','steps':[('main.py::leaf','call')]}]
        record('trace_exact',tr_code==0 and paths==expected_paths,{'exit':tr_code,'paths':paths},'one exact ordered main-to-leaf call path and no extras')
        # Report files live outside the selected source so they cannot perturb
        # discovery/input identity during the before/after loop.
        before=sandbox/'before.json';after=sandbox/'after.json';bad=sandbox/'incomplete.json'
        before.write_text(json.dumps(clean));bad.write_text(json.dumps(incomplete))
        (scopes['clean']/'main.py').write_text('# changed clean module\n')
        after_code,after_value,_=analyze('clean','after');after.write_text(json.dumps(after_value))
        diff_code,diff,_=invoke('diff-comparable',['diff',before,after,'--format','json','--quiet'])
        lists=['entities_added','entities_removed','entities_changed','diagnostics_new','diagnostics_resolved']
        record('comparable_diff',after_code==0 and diff_code==0 and isinstance(diff,dict) and all(diff.get(k)==[] for k in lists) and diff.get('has_regressions') is False,{'exit':diff_code,'diff':diff},'complete same-scope comment edit comparable, no structural changes')
        bad_code,bad_diff,_=invoke('diff-incomplete',['diff',before,bad,'--format','json','--quiet'])
        record('incomparable_diff',bad_code==3 and (not isinstance(bad_diff,dict) or bad_diff.get('diagnostics_resolved',[])==[]),{'exit':bad_code,'diff':bad_diff},'exit 3 for incomplete input; never assert resolution')
        sarif_code,sarif,_=invoke('sarif-incomplete',['analyze',scopes['incomplete'],'--format','sarif','--quiet'])
        runs=sarif.get('runs',[]) if isinstance(sarif,dict) else []
        inv=runs[0].get('invocations',[]) if len(runs)==1 else []
        record('sarif_incomplete',sarif_code==3 and len(inv)==1 and inv[0].get('executionSuccessful') is False and bool(inv[0].get('toolExecutionNotifications')),{'exit':sarif_code,'invocations':inv},'exit 3; unsuccessful SARIF invocation with visible loss notifications')
        continuation_code,page,_=analyze('findings','continuation',extra=['--limit','1','--offset','0'])
        ca=page.get('authority',{}) if isinstance(page,dict) else {}
        continuation=ca.get('continuation',{})
        second_code,second,_=analyze('findings','continuation-second',extra=['--limit','1','--offset','1'])
        sa=second.get('authority',{}) if isinstance(second,dict) else {}
        same=lambda a:private_contract(a,expected) and all(a[k]==fa[k] for k in IDENTITY+['report_identity'])
        first_meta={'full':2,'matched_total':2,'shown':1,'truncated':True,'next_cursor':1}
        second_meta={'full':2,'matched_total':2,'shown':1,'truncated':True,'next_cursor':None}
        indices=ca.get('display',{}).get('indices',[])+sa.get('display',{}).get('indices',[])
        record('continuation',continuation_code==second_code==2 and same(ca) and same(sa) and private_legacy_contract(page,expected) and private_legacy_contract(second,expected) and continuation==first_meta and sa.get('continuation')==second_meta and indices==[0,1] and page.get('diagnostics')==findings.get('diagnostics',[])[:1] and second.get('diagnostics')==findings.get('diagnostics',[])[1:2] and page.get('displayed_findings_total')==1 and second.get('displayed_findings_total')==0,{'first_exit':continuation_code,'second_exit':second_code,'first_metadata':continuation,'second_metadata':sa.get('continuation'),'indices':indices},'actual first and second pages retain full two-cause catalog, all authority identity fields and nonnull Graph')

        # Native handoff exercises a separate repository command against the
        # changed file, then checks the analyzer gate; FLOWSpec cannot replace it.
        validation=['python3','-m','py_compile',str(scopes['clean']/'main.py')]
        native=subprocess.run(validation,cwd=sandbox,env=env,capture_output=True,timeout=20)
        (out/'native-validation.stdout').write_bytes(native.stdout);(out/'native-validation.stderr').write_bytes(native.stderr)
        result['native_validation']={'argv':validation,'exit':native.returncode,'scope':'controlled Python fixture only; target repository still owns its real native suite'}
        gate_code,gate,_=invoke('structural-gate',['diagnose',scopes['clean'],'--gate-severity','info','--gate-confidence','low','--format','json','--quiet'])
        record('native_validation_handoff',native.returncode==0 and gate_code==0 and isinstance(gate,dict) and gate.get('authority',{}).get('execution')=='complete',{'native_exit':native.returncode,'structural_exit':gate_code},'native fixture validation followed by independent complete structural gate')
        result['support_limits']=[
            'Development archive is not a downloaded/reviewed release artifact.',
            'Trace currently emits a focused legacy path array rather than AnalysisReport authority.',
            'No CLI finding-ID diagnose selector is exposed; exact catalog matching is tested instead.',
            'Controlled Python fixtures do not qualify thirteen-by-four diagnostic semantics or target-repository validation.',
        ]
    result['status']='ACCEPT_DEVELOPMENTAL_WORKFLOW' if not result['holds'] else 'REWORK_WORKFLOW'
    (out/'workflow.json').write_text(json.dumps(result,sort_keys=True,indent=2)+'\n')
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--archive',type=Path,required=True);p.add_argument('--out',type=Path,required=True);args=p.parse_args()
    print('import:',Path(__file__).resolve(),file=sys.stderr)
    try:
        result=run_workflow(args.archive,args.out)
        print(json.dumps({'status':result['status'],'earned':result['earned'],'holds':result['holds']}))
        return 0 if not result['holds'] else 1
    except (ValueError,OSError,KeyError,subprocess.SubprocessError,zipfile.BadZipFile) as exc:
        args.out.mkdir(parents=True,exist_ok=True)
        result={'schema':'flowspec.skill-workflow.v1','status':'HOLD_HARNESS','earned':[],'holds':[{'operation':'harness','reason':str(exc)}],'release_qualified':False}
        (args.out/'workflow.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));return 1
if __name__=='__main__':sys.exit(main())
