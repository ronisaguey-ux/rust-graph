module Broken where

import Data.Char (isDigit)

data Good = Good Int

parse :: String -> Int
parse text = go text 0
  where
    go [] acc = acc
    go (c:cs) acc = go cs (acc + 1)

broken :: String -> Int
broken text = "unterminated string
