classdef class_definition
  properties
    Value
  end

  methods
    function obj = class_definition(value)
      obj.Value = value;
    end

    function value = doubled(obj)
      value = 2 * obj.Value;
    end
  end
end
