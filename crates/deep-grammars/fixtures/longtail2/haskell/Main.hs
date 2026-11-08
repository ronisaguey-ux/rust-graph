{-# LANGUAGE OverloadedStrings #-}

module Main (main, parsePort) where

import Data.Char (isDigit, ord)
import qualified Data.Map as M
import Control.Monad (forM_)

data Mode = Debug | Release | Verbose Int
  deriving (Show, Eq)

newtype Port = Port { unport :: Int }
  deriving (Show)

type Name = String

class Describe a where
  describe :: a -> String
  describe _ = "thing"

instance Describe Mode where
  describe Debug = "debug"
  describe Release = "release"
  describe (Verbose n) = "verbose"

parsePort :: String -> Int
parsePort text = go text 0
  where
    go [] acc = acc
    go (c:cs) acc =
      if isDigit c
      then go cs (acc * 10 + (ord c - ord '0'))
      else acc

main :: IO ()
main = do
  let port = parsePort "8080"
  let modes = [Debug, Release]
  mapM_ print modes
  case port of
    0 -> putStrLn "zero"
    _ -> putStrLn ("port " ++ show port)
  forM_ modes $ \m -> print (describe m)
