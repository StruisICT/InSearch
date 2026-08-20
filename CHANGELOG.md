# Changelog

## 0.1.0 (2026-08-20)


### Features

* boolean queries, whole-word, case modes, match highlighting ([0fd7a74](https://github.com/StruisICT/InSearch/commit/0fd7a743fc1c09f0b7d7fa73d46772de7adc73bd))
* file filters (name, extension, size, modified date) ([09bb809](https://github.com/StruisICT/InSearch/commit/09bb80924399268b41b4b2518ff41e5db8bf627f))
* initial Inspector Fetchy — real-time content-aware file search ([e079f96](https://github.com/StruisICT/InSearch/commit/e079f967095bc27c957f294a1489e5c13852bda4))
* result actions — open/reveal, copy, export, in-place filter ([c1c5260](https://github.com/StruisICT/InSearch/commit/c1c52604bd30121fb12f3918dfaa7e72fd648004))
* session persistence and live progress ([01b0896](https://github.com/StruisICT/InSearch/commit/01b0896e78a81035869ea449e97d5a8993bb0ff8))


### Bug Fixes

* **ci:** allow platform-conditional dead Status variants on non-Windows ([b9299d5](https://github.com/StruisICT/InSearch/commit/b9299d5a3364cdf934756b87df91460e8a873596))
* **cli:** don't panic on a closed stdout pipe ([07d2b1c](https://github.com/StruisICT/InSearch/commit/07d2b1cd6e2db1b60ddfd9315af1d396d57a9a84))
* defer watch start to avoid scan race; robust context-menu status ([1d853d9](https://github.com/StruisICT/InSearch/commit/1d853d95179dababcf9662e6202dec59b1dc51a5))
* **security:** isolate parser panics, cap document size, bump zip ([f2c7380](https://github.com/StruisICT/InSearch/commit/f2c7380f815b777bae2bf549ed1a8f127ee2ede7))


### Miscellaneous Chores

* release 0.1.0 ([a1528db](https://github.com/StruisICT/InSearch/commit/a1528db0cd2e673c4f2e59eae8fe38b184c4d39b))
