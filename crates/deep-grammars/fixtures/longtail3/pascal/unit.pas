unit Geometry;

interface

uses SysUtils;

type
  TPoint = record
    X: Integer;
    Y: Integer;
  end;

var
  LastPoint: TPoint;

function MakePoint(X, Y: Integer): TPoint;
procedure Clear;

implementation

function MakePoint(X, Y: Integer): TPoint;
begin
  MakePoint.X := X;
  MakePoint.Y := Y;
end;

procedure Clear;
begin
  LastPoint.X := 0;
  LastPoint.Y := 0;
end;

end.
