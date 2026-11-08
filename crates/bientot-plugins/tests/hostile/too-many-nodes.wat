;; expect: fails
;; A well-formed card of 102 nodes; a card may have 64.
(module
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (global $out (mut i32) (i32.const 65536))
  (data (i32.const 0) "{\"slot\":\"flyout.card\",\"tree\":{\"type\":\"card\",\"lines\":[{\"type\":\"row\",\"children\":[")
  (data (i32.const 200) "{\"type\":\"text\",\"key\":\"title\"},")
  (data (i32.const 300) "]}]}}")
  (func $put (param $from i32) (param $len i32)
    (memory.copy (global.get $out) (local.get $from) (local.get $len))
    (global.set $out (i32.add (global.get $out) (local.get $len))))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (local $i i32)
    (global.set $out (i32.const 65536))
    (call $put (i32.const 0) (i32.const 79))
    (loop $more
      (call $put (i32.const 200) (i32.const 30))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $more (i32.lt_u (local.get $i) (i32.const 99))))
    (call $put (i32.const 200) (i32.const 29))
    (call $put (i32.const 300) (i32.const 5))
    (drop (call $set (i32.const 65536) (i32.sub (global.get $out) (i32.const 65536))))
    (i64.const 0)))
