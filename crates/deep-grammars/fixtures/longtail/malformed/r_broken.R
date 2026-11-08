score <- function(path {
  files <- list.files(path)
  sum(files)
}

result <- score("/tmp"
