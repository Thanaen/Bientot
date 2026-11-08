;; expect: fails
;; A well-formed card nested 8 stacks deep, within the 32 levels of JSON a request may have; a card
;; may have 6 levels.
(module
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (global $out (mut i32) (i32.const 65536))
  (data (i32.const 0) "{\"slot\":\"flyout.card\",\"tree\":{\"type\":\"card\",\"lines\":[")
  (data (i32.const 100) "{\"type\":\"stack\",\"children\":[")
  (data (i32.const 200) "{\"type\":\"text\",\"key\":\"title\"}")
  (data (i32.const 300) "]}")
  (func $put (param $from i32) (param $len i32)
    (memory.copy (global.get $out) (local.get $from) (local.get $len))
    (global.set $out (i32.add (global.get $out) (local.get $len))))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (local $i i32)
    (global.set $out (i32.const 65536))
    (call $put (i32.const 0) (i32.const 53))
    (loop $open
      (call $put (i32.const 100) (i32.const 28))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $open (i32.lt_u (local.get $i) (i32.const 8))))
    (call $put (i32.const 200) (i32.const 29))
    (loop $close
      (call $put (i32.const 300) (i32.const 2))
      (local.set $i (i32.sub (local.get $i) (i32.const 1)))
      (br_if $close (i32.gt_u (local.get $i) (i32.const 0))))
    (call $put (i32.const 300) (i32.const 2))
    (call $put (i32.const 301) (i32.const 1))
    (drop (call $set (i32.const 65536) (i32.sub (global.get $out) (i32.const 65536))))
    (i64.const 0)))
