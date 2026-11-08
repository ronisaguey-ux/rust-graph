module Scan
  ( Rule (..)
  , scanText
  , defaultRules
  ) where

import Data.List (isInfixOf, nub)
import qualified Data.Text as T

data Rule = Rule { ruleName :: String
                 , rulePattern :: String
                 , ruleWeight :: Int
                 } deriving (Show, Eq)

defaultRules :: [Rule]
defaultRules =
  [ Rule "password" "passwd" 3
  , Rule "secret" "secret" 2
  , Rule "token" "token" 1
  ]

match :: Rule -> String -> Bool
match rule text = rulePattern rule `isInfixOf` text

scanText :: [Rule] -> String -> [(String, Int)]
scanText rules text = go rules 0
  where
    go [] _ = []
    go (r:rs) total
      | match r text = (ruleName r, total + ruleWeight r) : go rs total
      | otherwise = go rs total

names :: [Rule] -> [String]
names rules = nub (map ruleName rules)
