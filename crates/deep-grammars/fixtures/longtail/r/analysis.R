# Score a repository by size and churn.
library(jsonlite)

defaults <- list(
  min_lines = 10L,
  weight = 0.35,
  exclude = c(".git", "node_modules")
)

score_repo <- function(path, rules = defaults) {
  files <- list.files(path, recursive = TRUE, all.files = FALSE)
  files <- files[!grepl("\\.git", files)]
  total <- 0
  for (file in files) {
    full <- file.path(path, file)
    info <- file.info(full)
    if (is.na(info$size) || info$size < rules$min_lines) next
    total <- total + info$size * rules$weight
  }
  return(total)
}

summarise <- function(results) {
  values <- vapply(results, function(x) x$score, numeric(1))
  list(
    n = length(values),
    mean = mean(values),
    sd = if (length(values) > 1) sd(values) else 0,
    top = results[[which.max(values)]]
  )
}
