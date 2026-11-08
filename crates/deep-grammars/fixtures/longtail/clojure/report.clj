(ns scanner.report
  (:require [clojure.pprint :refer [pprint]]))

(defn row [name value]
  (format "| %-16s | %10d |" name value))

(defn render [summary]
  (let [lines (concat
                ["## Scanner summary" ""]
                ["| metric           |      value |"]
                [(row "files" (:count summary))]
                [(row "bytes" (:bytes summary))])]
    (clojure.string/join "\n" lines)))

(defn emit [summary target]
  (spit target (render summary))
  (pprint summary))
