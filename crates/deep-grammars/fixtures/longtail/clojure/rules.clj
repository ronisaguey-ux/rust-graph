(ns scanner.rules)

(def rules
  [{:name "size" :max 1000000}
   {:name "depth" :max 8}
   {:name "vendor" :pattern #"node_modules|target"}])

(defn size-ok? [file]
  (<= (.length file) 1000000))

(defn depth-ok? [path]
  (<= (count (str/split path #"/")) 8))

(defn applies? [{:keys [name max]} file]
  (case name
    "size" (size-ok? file)
    "depth" (depth-ok? (.getPath file))
    true))

(defn evaluate [files]
  (->> files
       (filter #(every? (fn [rule] (applies? rule %)) rules))
       (mapv #(.getPath %))))
