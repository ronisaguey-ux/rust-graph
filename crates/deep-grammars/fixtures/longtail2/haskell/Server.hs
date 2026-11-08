module Server where

import qualified Data.Map as M
import Control.Monad (when, unless)

data Config = Config { host :: String
                     , port :: Int
                     , verbose :: Bool
                     }

defaultConfig :: Config
defaultConfig = Config { host = "localhost"
                       , port = 8080
                       , verbose = False
                       }

describe :: Config -> String
describe config = host config ++ ":" ++ show (port config)

handle :: Config -> String -> String
handle config path =
  case path of
    "/health" -> "ok"
    "/version" -> "1.0"
    _ -> if verbose config then "debug: " ++ path else "not found"

route :: M.Map String (Config -> String)
route = M.fromList [ ("/health", \c -> handle c "/health")
                   , ("/version", \c -> handle c "/version")
                   ]

serve :: Config -> [String] -> [String]
serve config paths = map (handle config) paths

main :: IO ()
main = do
  let config = defaultConfig { verbose = True }
  when (verbose config) $ putStrLn "verbose"
  unless (null (serve config ["/health"])) $ putStrLn (describe config)
