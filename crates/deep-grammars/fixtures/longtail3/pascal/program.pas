program Greet;

uses SysUtils;

const
  MaxItems = 10;
  Greeting = 'hello';

var
  Count: Integer;
  Names: array[1..MaxItems] of string;

procedure SayHello(Name: string);
begin
  WriteLn(Greeting, ' ', Name);
end;

function Add(A, B: Integer): Integer;
begin
  Add := A + B;
end;

begin
  Count := Add(1, 2);
  SayHello('world');
  if Count > 0 then
    WriteLn('positive')
  else
    WriteLn('zero');
  while Count > 0 do
    Count := Count - 1;
  for Count := 1 to MaxItems do
    WriteLn(Count);
end.
