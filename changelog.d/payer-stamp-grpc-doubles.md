### PayerStamp usage reports work over gRPC A2A

- **Fixed:** a PayerStamp report sent over the gRPC A2A binding, whose metadata numbers all arrive as doubles, is no longer refused: whole doubles in the exact range are read as integers, while fractional or out-of-range numbers are still refused (#1661).
