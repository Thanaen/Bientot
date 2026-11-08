;; expect: lives
;; Writes 300 lines per event. The host keeps 60 a minute and counts the rest; this is no failure.
(module
  (import "bt:log@1" "write" (func $log (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (data (i32.const 0) "{\"level\":\"info\",\"text\":\"flood\"}")
  (func (export "bt_handle") (param i32 i32) (result i64)
    (local $i i32)
    (loop $more
      (drop (call $log (i32.const 0) (i32.const 31)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $more (i32.lt_u (local.get $i) (i32.const 300))))
    (i64.const 0)))
