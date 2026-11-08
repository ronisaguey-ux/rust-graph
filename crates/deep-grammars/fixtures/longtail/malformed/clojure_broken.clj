(ns scanner.broken)

(defn scan [root]
  (let [files (walk root]
    (count files)))
