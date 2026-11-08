program Shapes;

uses Math;

type
  TColor = (Red, Green, Blue);
  TPoint = record
    X: Integer;
    Y: Integer;
  end;
  TLine = record
    Start: TPoint;
    Finish: TPoint;
  end;

var
  Origin: TPoint;
  Line: TLine;

function Distance(A, B: TPoint): Integer;
begin
  Distance := Abs(A.X - B.X) + Abs(A.Y - B.Y);
end;

procedure Reset(var P: TPoint);
begin
  P.X := 0;
  P.Y := 0;
end;

begin
  Reset(Origin);
  Line.Start := Origin;
  case Distance(Line.Start, Line.Finish) of
    0: WriteLn('same');
    1: WriteLn('close');
  else
    WriteLn('far');
  end;
end.
