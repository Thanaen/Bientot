;; expect: lives
;; Not hostile: the runner sends it commands while the others misbehave and times its card.
;; Each event shows the other of two cards, so that every answer is a change.
(module
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (global $flip (mut i32) (i32.const 0))
  (data (i32.const 0) "{\"slot\":\"flyout.card\",\"tree\":{\"type\":\"card\",\"lines\":[{\"type\":\"text\",\"key\":\"title\"}]}}")
  (data (i32.const 200) "{\"slot\":\"flyout.card\",\"tree\":{\"type\":\"card\",\"lines\":[{\"type\":\"text\",\"key\":\"title\",\"role\":\"muted\"}]}}")
  (func (export "bt_handle") (param i32 i32) (result i64)
    (global.set $flip (i32.xor (global.get $flip) (i32.const 1)))
    (if (global.get $flip)
      (then (drop (call $set (i32.const 0) (i32.const 85))))
      (else (drop (call $set (i32.const 200) (i32.const 100)))))
    (i64.const 0)))
