// Soft-paint spike stills (throwaway): visits each variant's area (tags spa..), each view, logs "ETSHOT <name>" when it is in
// place; the driver outside the game takes the picture. The cameraman is hidden.
void Shot(object pc, string tag, vector p, float facing, float dist, float pitch, string name)
{
    object a = GetObjectByTag(tag);
    AssignCommand(pc, ClearAllActions());
    AssignCommand(pc, JumpToLocation(Location(a, p, facing)));
    DelayCommand(8.0, AssignCommand(pc, SetCameraFacing(facing, dist, pitch, CAMERA_TRANSITION_TYPE_SNAP)));
    DelayCommand(9.0, PrintString("ETSHOT " + name));
}
void main()
{
    object pc = GetFirstPC();
    if (!GetIsObjectValid(pc)) { DelayCommand(1.0, ExecuteScript("sp_tour", GetModule())); return; }
    if (GetLocalInt(GetModule(), "toured")) return;
    SetLocalInt(GetModule(), "toured", 1);
    ApplyEffectToObject(DURATION_TYPE_PERMANENT, SupernaturalEffect(EffectVisualEffect(VFX_DUR_CUTSCENE_INVISIBILITY)), pc);
    string vs = GetLocalString(GetModule(), "SP_ORDER");
    float t = 5.0; int i;
    for (i = 0; i < GetStringLength(vs); i++)
    {
        string v = GetSubString(vs, i, 1);
        string tag = "sp" + v;
        DelayCommand(t, Shot(pc, tag, Vector(12.0, 66.0, 0.0), 0.0, 7.0, 68.0, "_load-" + v)); t += 30.0;
        DelayCommand(t, Shot(pc, tag, Vector(12.0, 66.0, 0.0), 0.0, 7.0, 68.0, v + "-1-james")); t += 18.0;
        DelayCommand(t, Shot(pc, tag, Vector(26.0, 68.2, 0.0), 10.0, 4.0, 76.0, v + "-2-low")); t += 18.0;
        DelayCommand(t, Shot(pc, tag, Vector(28.0, 68.0, 0.0), 90.0, 16.0, 8.0, v + "-3-high")); t += 18.0;
        DelayCommand(t, Shot(pc, tag, Vector(60.0, 71.0, 0.0), 90.0, 30.0, 40.0, v + "-4-wide")); t += 18.0;
    }
    DelayCommand(t, PrintString("ETSHOT done"));
}
