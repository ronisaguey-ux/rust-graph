(ns scanner.core
  (:require [clojure.string :as str]
            [clojure.java.io :as io]))

(def default-depth 12)

(defn walk
  "List files under a root, up to a depth."
  ([root] (walk root default-depth))
  ([root depth]
   (let [file (io/file root)]
     (if (or (nil? file) (zero? depth))
       []
       (mapcat (fn [entry]
                 (if (.isDirectory entry)
                   (walk (.getPath entry) (dec depth))
                   [(.getPath entry)]))
               (or (.listFiles file) []))))))

(defmacro with-timing [& body]
  `(let [started# (System/nanoTime)
         result# (do ~@body)]
     {:result result# :elapsed (- (System/nanoTime) started#)}))

(defn scan [root]
  (let [files (walk root)
        total (reduce + 0 (map #(.length (io/file %)) files))]
    {:count (count files) :bytes total}))

(defn -main [& args]
  (println (scan (or (first args) "."))))

(-main ".")
