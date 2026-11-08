;; expect: fails
;; Exports memory and bt_abi only.
(module
  (memory (export "memory") 1)
  (func (export "bt_abi") (result i32) i32.const 1))
