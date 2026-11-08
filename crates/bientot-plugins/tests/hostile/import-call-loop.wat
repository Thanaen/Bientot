;; expect: fails
;; Calls bt:time@1 now without end; each call charges fuel although the host does the work.
(module
  (import "bt:time@1" "now" (func $now (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (data (i32.const 0) "{}")
  (func (export "bt_handle") (param i32 i32) (result i64)
    (loop $more
      (drop (call $now (i32.const 0) (i32.const 2)))
      (br $more))
    (i64.const 0)))
