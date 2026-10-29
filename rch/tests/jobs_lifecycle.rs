//! Exercise the real jobs CLI against an isolated Unix-socket peer and durable
//! lease files. Ordinary tests do not execute compilers. The opt-in engineering
//! fixture below uses real local tools and simulated SSH in a private namespace;
//! no test connects to an installed daemon or worker.
#![cfg(target_os = "linux")]

use rch_common::job_identity::{DurableJobLease, JobIdentity};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Real CLI, source claims, process groups, completion receipts and byte streams.
/// Only worker selection (a socket peer) and SSH (a local shell transport) are
/// substituted. A private /tmp keeps the worker claim registry away from any
/// installation. Run explicitly on a Linux engineering host with mount rights.
#[test]
#[ignore = "requires mount rights, Python 3 and RCH_TEST_NATIVE_RUST_BIN pointing at a native stable toolchain"]
fn source_mount_native_and_shared_terminal_lifecycle() {
    let root = tempfile::Builder::new()
        .prefix("rch-terminal-")
        .tempdir_in("/var/tmp")
        .unwrap()
        .keep();
    let binary = root.join("rch");
    fs::copy(
        std::env::var("RCH_TEST_SUBJECT_BINARY")
            .unwrap_or_else(|_| env!("CARGO_BIN_EXE_rch").into()),
        &binary,
    )
    .unwrap();
    let output = Command::new("unshare")
        .args(["--mount", "--propagation", "private", "sh", "-c", "mount -t tmpfs -o size=256m tmpfs /tmp && exec python3 -c \"$1\" \"$2\" \"$3\"", "rch-test"])
        .arg(r#"
import json, os, pathlib, signal, socket, subprocess, sys, threading, time, urllib.parse
root = pathlib.Path(sys.argv[1])
native = pathlib.Path(sys.argv[2])
for name in ['bin','home','config','state','source/project','transfer','tmp','ordinary','private']:
    (root/name).mkdir(parents=True, exist_ok=True)
ssh = root/'bin/ssh'
ssh.write_text('''#!/usr/bin/python3
import os,sys,shlex
args=sys.argv[1:]
i=0
while i<len(args):
    if args[i] in ['-o','-i','-p','-l','-S']: i+=2
    elif args[i].startswith('-'): i+=1
    else: break
assert args[i] in ['root@rch-fixture','rch-fixture'],args
command=' '.join(args[i+1:])
if os.environ.get('FAIL_CLEANUP') and command=='sh -s':
    sys.exit(255)
if os.environ.get('MISSING_RECEIPT') and 'cat -- ' in command and '.done' in command:
    sys.exit(0)
parsed=shlex.split(command)
if parsed[:2] in [['sh','-c'],['sh','-lc'],['bash','-c'],['bash','-lc']]:
    # Only inspect tokens for this adapter's fault injection. Shell comments
    # can contain unmatched quotes; they are not arguments to tokenize. The
    # command actually executed below is still the unchanged authored string.
    parsed=shlex.split(parsed[2],comments=True)
if parsed[-2:-1]==['sh'] and parsed[-1].endswith('.done.launch'):
    import json
    with open(os.environ['RCH_FIXTURE_LAUNCH_LOG'],'a') as log:
        log.write(json.dumps({'argv':sys.argv,'max_argument_bytes':max(len(a.encode()) for a in sys.argv),'command':command})+'\\n')
    if os.environ.get('FAIL_AFTER_RAW_SPAWN'):
        sys.exit(255)
if os.environ.get('FAIL_RTK_COLLECTION') and parsed[4:5]==['manifest']:
    sys.exit(255)
if os.environ.get('WORKER_ORDINARY_PATH'):
    os.environ['PATH']=os.environ['WORKER_ORDINARY_PATH']
os.environ['RCH_FIXTURE_TRANSPORT']='worker'
os.execv('/bin/sh',['sh','-c',command])
''')
ssh.chmod(0o755)
config = f'''[general]
socket_path = "/tmp/terminal.sock"
[transfer]
remote_base = "{root}/transfer"
[jobs]
shared_workspaces = [{{worker="fixture",controller_root="{root}/source",worker_root="{root}/source"}}]
rtk_tee_root = "{root}/controller-output"
[execution.storage]
tmp_root = "{root}/tmp"
cache_root = "{root}/worker-cache"
home_root = "{root}/worker-home"
tmp_mode = "private_mount"
'''
(root/'config/config.toml').write_text(config)
(root/'config/workers.toml').write_text('workers = []\n')
env = dict(PATH=f'{root}/bin:/usr/bin:/bin', HOME=str(root/'home'),
    CARGO_HOME=str(root/'controller-cache/cargo-home'),
    XDG_CONFIG_HOME=str(root/'config'), RCH_CONFIG_DIR=str(root/'config'),
    RCH_STATE_HOME=str(root/'state'), RCH_SOCKET_PATH='/tmp/terminal.sock',
    RCH_FIXTURE_TRANSPORT='controller',
    RCH_FIXTURE_LAUNCH_LOG=str(root/'raw-launches.jsonl'),
    RCH_NO_SELF_HEALING='1', RCH_NO_UPDATE_CHECK='1', RCH_REQUIRE_REMOTE='1',
    RCH_WORKER='fixture', RCH_LOG_LEVEL='error', NO_COLOR='1')
listener=socket.socket(socket.AF_UNIX); listener.bind('/tmp/terminal.sock'); listener.listen(); listener.settimeout(.1)
requests=[]; releases=[]; selections=[]; errors=[]; done=False
def serve():
    while not done:
        try: stream,_=listener.accept()
        except TimeoutError: continue
        try:
            stream.settimeout(3); data=b''
            while b'\n' not in data:
                block=stream.recv(8192)
                if not block: break
                data+=block
            request=data.decode().splitlines()[0]; requests.append(request)
            route=request.split(' ',1)[1]; query=urllib.parse.parse_qs(urllib.parse.urlsplit(route).query)
            reply={'status':'ok'}
            endpoint=urllib.parse.urlsplit(route).path
            if endpoint in ('/select-worker','/select-toolchain-environment'):
                # Mirror both current selection protocols in this explicitly
                # simulated peer. Native requests carry admission-bound tool
                # environment; do not answer them with a generic success object.
                if endpoint=='/select-toolchain-environment':
                    assert 'toolchain_environment' in query
                    assert isinstance(json.loads(query['toolchain_environment'][0]),dict)
                selections.append(query)
                reply={'worker':{'id':'fixture','host':'rch-fixture','user':'root','identity_file':'/unused','slots_available':2,'speed_score':1.0},'reason':'success','build_id':8000+len(selections)}
            if route.startswith('/release-worker'): releases.append(query)
            stream.sendall(b'HTTP/1.1 200 OK\r\n\r\n'+json.dumps(reply).encode())
        except Exception as e: errors.append(repr(e))
        finally: stream.close()
thread=threading.Thread(target=serve,daemon=True); thread.start()
code='import os,subprocess,time; p=subprocess.Popen(["sleep","30"]); open("pids","w").write(str(os.getpid())+" "+str(p.pid)); open("runs","a").write("x"); os.write(1,b"out\\x00\\xff"); os.write(2,b"err\\x00\\xfe"); time.sleep(30)'
try:
    for name,expected,outcome in [('timeout',124,'deadline_exceeded'),('interrupt',130,'interrupted'),('exit137',137,'completed')]:
        cwd=root/'source/project'/name; cwd.mkdir()
        result=root/(name+'.json')
        command=[str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(cwd),'--timeout','1s' if name=='timeout' else '20s','--json-result',str(result),'--','/usr/bin/python3','-c', code if name!='exit137' else 'import sys; sys.exit(137)']
        started=time.monotonic(); p=subprocess.Popen(command,env=env,cwd=cwd,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        if name=='interrupt':
            while not (cwd/'pids').exists():
                assert p.poll() is None
                assert time.monotonic()-started<10
                time.sleep(.02)
            p.send_signal(signal.SIGINT)
        stdout,stderr=p.communicate(timeout=20)
        elapsed=time.monotonic()-started
        (root/(name+'.stdout')).write_bytes(stdout); (root/(name+'.stderr')).write_bytes(stderr)
        assert p.returncode==expected,(name,p.returncode,stdout,stderr)
        record=json.loads(result.read_text()); assert record['outcome']==outcome,record
        assert record['remote_exit_code']==expected and record['worker_id']=='fixture',record
        assert elapsed<10,(name,elapsed)
        assert len(releases)==len(selections)==len([p for p in root.glob('*.json')])
        assert releases[-1]['exit_code']==[str(expected)],releases
        assert releases[-1]['build_id']==[str(8000+len(selections))]
        assert releases[-1]['local_wrapper_id']==selections[-1]['local_wrapper_id']
        lease=json.loads((root/'state/job-leases'/(releases[-1]['local_wrapper_id'][0]+'.json')).read_text())
        assert lease['terminal_acknowledged'] and lease['exit_code']==expected,lease
        assert lease['recovery']['retired'] and lease['recovery']['sources_released'],lease
        if name!='exit137':
            assert stdout==b'out\x00\xff',stdout
            assert b'err\x00\xfe' in stderr,stderr
            assert (cwd/'runs').read_bytes()==b'x'
            for pid in (cwd/'pids').read_text().split():
                stat=pathlib.Path('/proc')/pid/'stat'
                assert not stat.exists() or stat.read_text().rsplit(') ',1)[1][0] in 'ZX',pid
        print(name, 'PASS', round(elapsed,3), record)
    # Positive per-call contract through the actual execution/receipt path.
    # SSH and admission are the same explicitly simulated adapters above;
    # the real workload asserts it reached that adapter, not local fallback.
    strict_work=root/'source/project/strict-call'; strict_work.mkdir()
    strict_result=root/'strict-call.json'; strict_payload=b'literal\x00\xffstdin'
    strict_code='import os,sys; assert os.environ["RCH_FIXTURE_TRANSPORT"]=="worker"; data=sys.stdin.buffer.read(); open("runs","ab").write(b"x"); sys.stdout.buffer.write(data); sys.stderr.buffer.write(b"err\\x00\\xfe"); sys.exit(23)'
    before=len(selections); before_release=len(releases)
    strict_run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--require-remote','--cwd',str(strict_work),'--json-result',str(strict_result),'--','/usr/bin/python3','-c',strict_code],env=dict(env,RCH_REQUIRE_REMOTE='0'),cwd=strict_work,input=strict_payload,capture_output=True,timeout=20)
    assert strict_run.returncode==23,(strict_run.returncode,strict_run.stderr)
    assert strict_run.stdout==strict_payload,strict_run.stdout
    assert b'err\x00\xfe' in strict_run.stderr,strict_run.stderr
    strict_record=json.loads(strict_result.read_text())
    assert strict_record['outcome']=='completed' and strict_record['remote_exit_code']==23 and strict_record['worker_id']=='fixture',strict_record
    assert (strict_work/'runs').read_bytes()==b'x'
    assert len(selections)==before+1 and len(releases)==before_release+1
    assert releases[-1]['exit_code']==['23']
    assert releases[-1]['local_wrapper_id']==selections[-1]['local_wrapper_id']
    print('per-call-strict-positive PASS: exact binary streams, worker contract, one execution and one release')
    for name,variable,workload in [('unverified','FAIL_CLEANUP',code),('missing','MISSING_RECEIPT','open("runs","a").write("x")')]:
        cwd=root/'source/project'/name; cwd.mkdir()
        result=root/(name+'.json'); before=len(releases)
        command=[str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(cwd),'--timeout','1s','--json-result',str(result),'--','/usr/bin/python3','-c',workload]
        p=subprocess.run(command,env=dict(env,**{variable:'1'}),cwd=cwd,capture_output=True,timeout=15)
        (root/(name+'.stdout')).write_bytes(p.stdout); (root/(name+'.stderr')).write_bytes(p.stderr)
        assert p.returncode==1,(name,p.returncode,p.stderr)
        record=json.loads(result.read_text()); assert record['outcome']=='transport_error',record
        assert len(releases)==before,(name,releases)
        assert (cwd/'runs').read_bytes()==b'x'
        lease=json.loads((root/'state/job-leases'/(selections[-1]['local_wrapper_id'][0]+'.json')).read_text())
        assert not lease['terminal_acknowledged'],lease
        print(name,'PASS: ownership retained, no replay',record)
    version=subprocess.check_output([str(native/'rustc'),'--version'],text=True).split()[1]
    assert '-' not in version,version
    for tool in ['rustc','cargo','rustdoc']:
        (root/'ordinary'/tool).symlink_to(native/tool)
    for tool in ['cargo-clippy','clippy-driver']:
        (root/'private'/tool).symlink_to(native/tool)
    ordinary_path=f'{root}/ordinary:/usr/bin:/bin'
    native_path=f'{root}/private:{ordinary_path}'
    negative=subprocess.run(['cargo','clippy','--version'],env=dict(env,PATH=ordinary_path,CARGO_HOME=str(root/'cargo-home')),capture_output=True)
    assert negative.returncode!=0 and b'no such command: `clippy`' in negative.stderr,negative
    env['WORKER_ORDINARY_PATH']=ordinary_path
    config+=f'''[path_topology]
source_root = "{root}/source"
canonical_root = "{root}/mirror"
alias_root = "{root}/mirror-alias"
[compilation]
native_toolchain = "{version}"
build_slots = 1
test_slots = 1
check_slots = 1
[environment.remote]
PATH = "{native_path}"
CARGO_HOME = "{root}/cargo-home"
TMPDIR = "/tmp"
'''
    (root/'config/config.toml').write_text(config)
    env['PATH']=f'{root}/bin:{native_path}'
    env['RCH_REAL_CARGO']=str(native/'cargo')
    env['RCH_SHIM_REAL_CARGO_CLIPPY']=str(native/'cargo-clippy')
    env['RCH_LOG_LEVEL']='debug'
    env['RCH_VERBOSE']='1'
    (root/'bin/rch').symlink_to(root/'rch')
    cwd=root/'source/project/native'; (cwd/'src').mkdir(parents=True)
    dep=root/'source/project/native-dep'; (dep/'src').mkdir(parents=True)
    (dep/'Cargo.toml').write_text('[package]\nname="native_dep"\nversion="0.1.0"\nedition="2021"\n')
    (dep/'src/lib.rs').write_text('pub fn value() -> u32 { 7 }\n')
    (cwd/'Cargo.toml').write_text('[package]\nname="native_smoke"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nnative_dep={path="../native-dep"}\n')
    (cwd/'build.rs').write_text('fn main() { assert!(std::env::current_dir().unwrap().to_str().unwrap().contains("/mirror/project/native")); }\n')
    (cwd/'src/lib.rs').write_text('#[test] fn mirrored() { assert_eq!(native_dep::value(),7); assert!(std::env::current_dir().unwrap().to_str().unwrap().contains("/mirror/project/native")); }\n')
    subprocess.run([str(native/'cargo'),'generate-lockfile','--offline'],env=dict(env,CARGO_HOME=str(root/'cargo-home')),cwd=cwd,check=True,capture_output=True)
    for d in [cwd,dep]:
        subprocess.run(['/usr/bin/git','init','-b','main'],cwd=d,check=True,capture_output=True)
        subprocess.run(['/usr/bin/git','add','.'],cwd=d,check=True,capture_output=True)
        subprocess.run(['/usr/bin/git','-c','user.name=RCH fixture','-c','user.email=fixture@example.invalid','commit','-m','fixture'],cwd=d,check=True,capture_output=True)
        subprocess.run(['/usr/bin/git','remote','add','origin','https://example.invalid/fixture.git'],cwd=d,check=True,capture_output=True)
    inputs={p:p.read_bytes() for d in [cwd,dep] for p in d.rglob('*') if p.is_file() and '.git' not in p.parts}
    install=subprocess.run([str(root/'rch'),'--no-self-healing','shim','install','--no-toolchains'],env=env,cwd=cwd,capture_output=True)
    assert install.returncode==0,install.stderr
    shim=root/'home/.rch/shims'
    for name,argv in [('default-cargo',[str(root/'rch'),'--no-self-healing','--json','exec','--','cargo','test','--offline','--locked']),('strict-cargo',[str(root/'rch'),'--no-self-healing','--json','exec','--require-remote','--','cargo','test','--offline','--locked']),('cargo-shim',[str(shim/'cargo'),'test','--no-run','--offline','--locked']),('clippy-shim',[str(shim/'cargo-clippy'),'--offline','--locked','--','-D','warnings'])]:
        before=len(releases); started=time.monotonic()
        call_env=dict(env,RCH_REQUIRE_REMOTE='0') if name=='strict-cargo' else env
        run=subprocess.run(argv,env=call_env,cwd=cwd,capture_output=True,timeout=60)
        (root/(name+'.stdout')).write_bytes(run.stdout); (root/(name+'.stderr')).write_bytes(run.stderr)
        assert run.returncode==0,(name,run.returncode,run.stdout,run.stderr)
        assert len(releases)==before+1 and releases[-1]['exit_code']==['0'],(name,releases)
        admission_environment=json.loads(selections[-1]['toolchain_environment'][0])
        assert admission_environment['PATH']==native_path,admission_environment
        assert all(p.read_bytes()==content for p,content in inputs.items())
        assert (root/'mirror/project/native/Cargo.toml').read_bytes()==inputs[cwd/'Cargo.toml']
        assert (root/'mirror/project/native-dep/src/lib.rs').read_bytes()==inputs[dep/'src/lib.rs']
        if name=='cargo-shim':
            # Ordinary cargo test streams results/reports; --no-run is the
            # product's executable-return contract. Exercise both entry shapes.
            artifacts=[p for p in (cwd/'target/debug/deps').glob('native_smoke-*') if p.is_file() and os.access(p,os.X_OK)]
            assert artifacts,(name,'native test artifact was not returned')
            listed=subprocess.run([str(artifacts[0]),'--list'],capture_output=True,check=True)
            assert b'mirrored: test' in listed.stdout,listed.stdout
        print(name,'PASS',round(time.monotonic()-started,3),'native Rust',version)
    # Explicit shared execution uses the same private tool defaults with an
    # ordinary worker shell PATH that cannot find Clippy.
    shared=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(cwd),'--','cargo','clippy','--version'],env=env,cwd=cwd,capture_output=True,timeout=20)
    assert shared.returncode==0,(shared.returncode,shared.stdout,shared.stderr)
    assert b'clippy' in shared.stdout,shared.stdout
    print('shared-private-path PASS',shared.stdout.decode().strip())
    if os.environ.get('RCH_TEST_RTK'):
        import hashlib, shlex
        rtk=pathlib.Path(os.environ['RCH_TEST_RTK'])
        assert subprocess.check_output([str(rtk),'--version']).strip()==b'rtk 0.46.0'
        assert hashlib.sha256(rtk.read_bytes()).hexdigest()=='bbe12b786f4c17ed068340a4de1d8b8540d22671aa4aa2c966af163d889640f9'
        (root/'ordinary/rtk').symlink_to(rtk)
        (root/'ordinary/go').symlink_to('/usr/local/go/bin/go')
        (root/'ordinary/bun').symlink_to('/root/.bun/bin/bun')
        import zipfile
        proxy=root/'module-proxy/fixture.invalid/dependency/@v';proxy.mkdir(parents=True)
        (proxy/'v0.0.1.mod').write_text('module fixture.invalid/dependency\n\ngo 1.23\n')
        (proxy/'v0.0.1.info').write_text('{"Version":"v0.0.1","Time":"2026-01-01T00:00:00Z"}')
        with zipfile.ZipFile(proxy/'v0.0.1.zip','w') as archive:
            archive.writestr('fixture.invalid/dependency@v0.0.1/go.mod',(proxy/'v0.0.1.mod').read_bytes())
            archive.writestr('fixture.invalid/dependency@v0.0.1/value.go','package dependency\nfunc Value() int { return 7 }\n')
        config+=f'GOPROXY = "file://{root}/module-proxy"\nGOSUMDB = "off"\nGOTOOLCHAIN = "local"\nGOFLAGS = "-mod=mod"\n'
        (root/'config/config.toml').write_text(config)
        cwd=root/'source/project/rtk';cwd.mkdir()
        (cwd/'go.mod').write_text('module fixture.invalid/rtk\n\ngo 1.23\nrequire fixture.invalid/dependency v0.0.1\n')
        (cwd/'main.go').write_text('package rtk\nimport "fixture.invalid/dependency"\nfunc Value() int { return dependency.Value() }\n')
        (cwd/'probe_test.go').write_text('''package rtk
import("testing";"os";"fmt";"os/exec";"encoding/json";"path/filepath")
func TestProbe(t *testing.T) {
 out,e:=exec.Command("go","env","-json","GOCACHE","GOMODCACHE","GOPATH").Output();if e!=nil{t.Fatal(e)}
 var paths map[string]string;json.Unmarshal(out,&paths)
 paths["tmp"]=os.TempDir();paths["cwd"],_=os.Getwd();paths["explicit"]=os.Getenv("EXPLICIT")
 if literal:=os.Getenv("RCH_ACCEPT_LITERAL");literal!="" {
  if literal!="literal value ; $HOME 中文"{t.Fatal("literal expanded",literal)}
  if os.Getenv("RCH_FIXTURE_TRANSPORT")!="worker"{t.Fatal("payload executed outside simulated worker")}
  paths["literal"]=literal;paths["transport"]=os.Getenv("RCH_FIXTURE_TRANSPORT")
  f,e:=os.OpenFile("literal-runs",os.O_CREATE|os.O_WRONLY|os.O_APPEND,0600);if e!=nil{t.Fatal(e)};f.WriteString("x");f.Close()
  fmt.Printf("LITERAL=%s\\n",literal)
 }
 if os.Getenv("EXPECT_SHARED")=="1" && paths["explicit"]!="literal value"{t.Fatal(paths)}
 if e:=os.WriteFile("resolved.json",mustJSON(paths),0600);e!=nil{t.Fatal(e)}
 f,e:=os.CreateTemp("","fixture-");if e!=nil{t.Fatal(e)};f.WriteString("scratch");f.Close()
 if !filepath.IsAbs(paths["GOCACHE"]){t.Fatal(paths)}
 for i:=0;i<80;i++{fmt.Printf("RAW-LINE-%03d hidden details for the failure witness\\n",i)}
 if os.Getenv("FAIL_FIXTURE")=="1"{t.Fatal("EXPECTED-FAILURE")}
}
func mustJSON(v any)[]byte{b,_:=json.Marshal(v);return b}
''')
        (cwd/'probe.test.js').write_text('import {test,expect} from "bun:test"; test("bounded",()=>{ for(let i=0;i<80;i++) console.log("BUN-RAW-"+i); expect(process.env.FAIL_FIXTURE).not.toBe("1"); });\n')
        (cwd/'entry.js').write_text('console.log("bundle");\n')
        import io,tarfile
        archive=root/'fixture-bun.tgz'
        with tarfile.open(archive,'w:gz') as tar:
            for name,data in [('package/package.json',b'{"name":"fixture-bun","version":"1.0.0","main":"index.js"}'),('package/index.js',b'export default 42;')]:
                entry=tarfile.TarInfo(name);entry.size=len(data);tar.addfile(entry,io.BytesIO(data))
        (cwd/'package.json').write_text(json.dumps({'dependencies':{'fixture-bun':str(archive)}}))
        (cwd/'Cargo.toml').write_text('[package]\nname="rtk_fixture"\nversion="0.1.0"\nedition="2021"\n[lib]\npath="lib.rs"\n')
        (cwd/'lib.rs').write_text('#[test] fn probe() { for i in 0..80 { println!("CARGO-RAW-{i}"); } assert!(std::env::var("FAIL_FIXTURE").unwrap_or_default() != "1"); }\n')
        subprocess.run([str(native/'cargo'),'generate-lockfile','--offline'],env=dict(env,CARGO_HOME=str(root/'worker-cache/cargo-home')),cwd=cwd,check=True,capture_output=True)
        subprocess.run(['/usr/bin/git','init','-b','main'],cwd=cwd,check=True,capture_output=True)
        subprocess.run(['/usr/bin/git','add','.'],cwd=cwd,check=True,capture_output=True)
        subprocess.run(['/usr/bin/git','-c','user.name=RCH fixture','-c','user.email=fixture@example.invalid','commit','-m','fixture'],cwd=cwd,check=True,capture_output=True)
        # A nested mount outside the source mapping no longer participates in
        # RTK output validation. It is confined to this fixture namespace.
        (root/'legacy-tee').mkdir()
        subprocess.run(['mount','-t','tmpfs','-o','size=1m','tmpfs',str(root/'legacy-tee')],check=True)
        env.update(GOTOOLCHAIN='local',GOPROXY='off',GOSUMDB='off')
        env['RCH_LOG_LEVEL']='error'
        cases=[
          ('run-c',"rtk run -c 'go test -v -count=1 ./...'",0),
          ('run-positional',"rtk run 'export EXPLICIT=\"literal value\"; /usr/local/go/bin/go test -v -count=1 ./...'",0),
          ('proxy-compound',"rtk proxy sh -c 'go test -v -count=1 ./... && bun test'",0),
          ('bun-cache',"rtk run -c 'bun install --ignore-scripts && bun pm cache && bun test'",0),
          ('filtered-go','rtk go test -count=1 ./...',1),
          ('filtered-cargo','rtk cargo test --offline --locked -- --nocapture',1),
          ('filtered-bun','rtk test bun test',1),
          ('absolute-bun','rtk test /root/.bun/bin/bun test',1),
          ('go-build','rtk go build ./...',0),
          ('cargo-build','rtk cargo build --offline --locked',0),
          ('bun-build','rtk err bun build entry.js --outdir dist',0),
        ]
        for name,source,fail in cases:
            event={'hook_event_name':'PreToolUse','tool_name':'Bash','tool_input':{'command':source},'cwd':str(cwd)}
            hook=subprocess.run([str(root/'rch'),'--no-self-healing'],input=json.dumps(event).encode(),env=env,cwd=cwd,capture_output=True,timeout=10)
            planned=json.loads(hook.stdout)['hookSpecificOutput']['updatedInput']['command']
            assert planned=='rch exec --verification --require-remote -- '+source,(source,planned)
            result=root/(name+'.json')
            argv=[str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(cwd),'--json-result',str(result),'--env',f'FAIL_FIXTURE={fail}','--env','EXPLICIT=literal value','--env','EXPECT_SHARED=1','--env',f'GOCACHE={root}/forbidden-cache','--env',f'TMPDIR={root}/forbidden-tmp','--',*shlex.split(source)]
            before=len(releases)
            run=subprocess.run(argv,env=env,cwd=cwd,capture_output=True,timeout=120)
            (root/(name+'.stdout')).write_bytes(run.stdout);(root/(name+'.stderr')).write_bytes(run.stderr)
            assert (run.returncode!=0)==bool(fail),(name,run.returncode,run.stdout,run.stderr)
            record=json.loads(result.read_text());assert record['outcome']=='completed',record
            assert len(releases)==before+1 and releases[-1]['exit_code']==[str(run.returncode)]
            logs=pathlib.Path(record['result_dirs'][0]['path']);assert logs.is_relative_to(root/'controller-output')
            assert (logs/'outer.stdout').read_bytes()==run.stdout
            raw=b''.join(p.read_bytes() for p in logs.glob('*.stdout'))+b''.join(p.read_bytes() for p in logs.glob('*.stderr'))
            if name.startswith('filtered-') or name=='absolute-bun':
                marker=b'RAW-LINE-000' if name=='filtered-go' else b'CARGO-RAW-0' if name=='filtered-cargo' else b'BUN-RAW-0'
                assert marker in raw,(name,raw)
                assert len(raw)>len(run.stdout),(name,'filter input not captured')
            if name=='filtered-go': assert b'RAW-LINE-000' not in run.stdout,run.stdout
            if name=='bun-cache':
                assert str(root/'worker-cache/bun').encode() in run.stdout,run.stdout
                assert any(p.is_file() and p.name=='index.js' for p in (root/'worker-cache/bun').rglob('*'))
            if name in ['run-c','run-positional','proxy-compound']:
                assert b'RAW-LINE-000' in run.stdout,(name,run.stdout)
            if 'go' in source:
                resolved=json.loads((cwd/'resolved.json').read_text())
                assert pathlib.Path(resolved['GOCACHE']).is_relative_to(root/'worker-cache/go-build'),resolved
                assert pathlib.Path(resolved['GOMODCACHE']).is_relative_to(root/'worker-cache/go-mod'),resolved
                assert resolved['tmp'].startswith(str(root/'tmp')),resolved
            assert not (root/'forbidden-cache').exists() and not (root/'forbidden-tmp').exists()
            print(name,'PASS',run.returncode,'raw bytes',len(raw))
        # Execute the hook's actual returned shell text, including the nested
        # quote concatenation from the live event. Only GOPROXY/cwd name our
        # isolated fixture; the live acceptance directory is never executed.
        inner=f"export GOTOOLCHAIN=local GOSUMDB=off GOFLAGS=-mod=mod GOPROXY=file://{root}/module-proxy RCH_ACCEPT_LITERAL='literal value ; $HOME 中文'; /usr/local/go/bin/go test -v -count=1 ./..."
        for index,prefix in enumerate(['rtk run -c ','rtk run ','rtk proxy sh -c ','rtk proxy bash -c '],1):
            source=prefix+shlex.quote(inner)
            event={'hook_event_name':'PreToolUse','tool_name':'Bash','tool_input':{'command':source,'timeout':120000,'description':'bounded literal/export hook execution'},'cwd':str(cwd)}
            before=len(selections);before_release=len(releases)
            hook=subprocess.run([str(root/'rch'),'--no-self-healing'],input=json.dumps(event).encode(),env=env,cwd=cwd,capture_output=True,timeout=10)
            assert hook.returncode==0,(hook.returncode,hook.stderr)
            updated=json.loads(hook.stdout)['hookSpecificOutput']['updatedInput']
            planned='rch exec --verification --require-remote -- '+source
            assert updated==dict(event['tool_input'],command=planned),updated
            assert len(selections)==before and len(releases)==before_release
            run=subprocess.run(['/bin/sh','-c',updated['command']],env=env,cwd=cwd,capture_output=True,timeout=120)
            (root/f'literal-{index}.stdout').write_bytes(run.stdout);(root/f'literal-{index}.stderr').write_bytes(run.stderr)
            assert run.returncode==0,(run.returncode,run.stdout,run.stderr)
            assert (cwd/'literal-runs').read_bytes()==b'x'*index
            assert len(selections)==before+1 and len(releases)==before_release+1
            assert releases[-1]['exit_code']==['0'],releases[-1]
            resolved=json.loads((cwd/'resolved.json').read_text())
            assert resolved['literal']=='literal value ; $HOME 中文' and resolved['transport']=='worker',resolved
            assert resolved['cwd']==str(cwd),resolved
            assert resolved['GOCACHE']==str(root/'worker-cache/go-build') and resolved['GOMODCACHE']==str(root/'worker-cache/go-mod'),resolved
            assert resolved['tmp'].startswith(str(root/'tmp')),resolved
            lease=json.loads((root/'state/job-leases'/(selections[-1]['local_wrapper_id'][0]+'.json')).read_text())
            assert lease['terminal_acknowledged'] and lease['exit_code']==0,lease
            logpath=next(line.split(': ',1)[1] for line in run.stderr.decode().splitlines() if line.startswith('[RCH] full RTK output: '))
            logs=pathlib.Path(logpath);assert logs.is_relative_to(root/'controller-output')
            assert (logs/'outer.stdout').read_bytes()==run.stdout
            assert 'LITERAL=literal value ; $HOME 中文'.encode() in run.stdout
            (root/f'literal-{index}.json').write_text(json.dumps({'event':event,'rewrite':updated,'resolved':resolved,'selection':selections[-1],'release':releases[-1],'lease':lease,'logs':logpath,'runs':index},ensure_ascii=False,indent=2))
            print('literal-hook',prefix.strip(),'PASS','once-only',index,'remote literal bytes',resolved['literal'].encode().hex())
        assert any(p.is_file() for p in (root/'worker-cache/go-build').rglob('*'))
        assert (root/'worker-cache/go-mod/fixture.invalid/dependency@v0.0.1/value.go').is_file()
        assert (root/'worker-cache/go-mod/cache/download/fixture.invalid/dependency/@v/v0.0.1.zip').is_file()
        assert not (root/'home/.cache/go-build').exists() and not (root/'home/go/pkg/mod').exists()
        assert (root/'worker-cache/cargo-home/.global-cache').is_file()
        assert (root/'controller-cache/cargo-home/.global-cache').is_file()
        assert not (root/'home/.cargo/.global-cache').exists() and not (root/'home/.bun/install/cache').exists()
        # Same placement through native Go mirror execution.
        native_go=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--','go','test','-v','-count=1','./...'],env=env,cwd=cwd,capture_output=True,timeout=120)
        assert native_go.returncode==0,(native_go.returncode,native_go.stdout,native_go.stderr)
        resolved=json.loads((root/'mirror/project/rtk/resolved.json').read_text())
        assert resolved['cwd']==str(root/'mirror/project/rtk'),resolved
        assert resolved['GOCACHE']==str(root/'worker-cache/go-build'),resolved
        assert resolved['GOMODCACHE']==str(root/'worker-cache/go-mod'),resolved
        print('native-go-resolved-cache PASS',resolved)
        # Capture below filtering is also complete for tiny (<500-byte) output.
        tiny=root/'ordinary/tiny';tiny.write_text('#!/bin/sh\nprintf "tiny\\000\\377"\nexit 7\n');tiny.chmod(0o700)
        result=root/'tiny.json'
        run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--json-result',str(result),'--','rtk','test',str(tiny)],env=env,cwd=cwd,capture_output=True,timeout=20)
        assert run.returncode==7,(run.returncode,run.stderr)
        logs=pathlib.Path(json.loads(result.read_text())['result_dirs'][0]['path'])
        assert any(p.read_bytes()==b'tiny\x00\xff' for p in logs.glob('*.stdout'))
        print('tiny-filtered-raw PASS')
        # A filter inside an RTK shell compound still captures an exact
        # executable path, including output above RTK's own 1 MiB tee limit.
        large=root/'ordinary/large';large.write_text('#!/usr/bin/python3\nimport sys\nsys.stdout.buffer.write(b"raw-line\\n"*150000+b"\\0\\xffEND")\nsys.exit(7)\n');large.chmod(0o700)
        result=root/'large.json'
        run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--json-result',str(result),'--','rtk','proxy','sh','-c',f'printf prefix; rtk test {shlex.quote(str(large))}'],env=env,cwd=cwd,capture_output=True,timeout=30)
        assert run.returncode==7,(run.returncode,run.stderr)
        logs=pathlib.Path(json.loads(result.read_text())['result_dirs'][0]['path'])
        assert any(p.read_bytes()==b'raw-line\n'*150000+b'\0\xffEND' for p in logs.glob('*.stdout'))
        print('large-nested-filter-absolute-raw PASS')
        # Actual installed RTK PATH shim owns the outer command too, so an
        # absolute workload inside rtk run cannot evade remote placement.
        install=subprocess.run([str(root/'rch'),'--no-self-healing','shim','install','--tools','rtk,go,node','--no-toolchains'],env=env,cwd=cwd,capture_output=True)
        assert install.returncode==0,install.stderr
        before=len(selections)
        run=subprocess.run([str(shim/'rtk'),'run','-c','/usr/local/go/bin/go test -count=1 ./...'],env=env,cwd=cwd,capture_output=True,timeout=120)
        assert run.returncode==0,(run.returncode,run.stdout,run.stderr)
        assert len(selections)==before+1 and 'rtk run' in selections[-1]['command'][0],selections[-1]
        before=len(selections)
        run=subprocess.run([str(shim/'rtk'),'run','-c','printf harmless; exit 7'],env=env,cwd=cwd,capture_output=True)
        assert run.returncode==7 and run.stdout==b'harmless' and len(selections)==before,(run, selections[-1])
        print('rtk-shim-build-and-ordinary PASS')
        # Regression for export + && inside the literal proxy shell. Both the
        # hook rewrite and the original command through generated PATH shims
        # must select ONE outer job. The real worker shell owns both tools,
        # exports, conditional evaluation and any intervening side effects.
        (cwd/'probe.test.cjs').write_text('''const {test}=require('node:test');
const assert=require('node:assert/strict'), fs=require('node:fs');
test('compound worker',()=>{
 assert.equal(process.env.RCH_FIXTURE_TRANSPORT,'worker');
 assert.equal(process.env.RCH_ACCEPT_LITERAL,'literal value ; $HOME 中文');
 const go=JSON.parse(fs.readFileSync('resolved.json'));
 assert.equal(go.cwd,process.cwd()); assert.equal(go.tmp,process.env.TMPDIR);
 assert.equal(go.GOCACHE,process.env.GOCACHE);
 fs.appendFileSync('node-runs','x'); console.log('NODE-COMPOUND-RAW');
 assert.notEqual(process.env.NODE_FAIL,'1');
});\n''')
        compound_inner=inner.replace('/usr/local/go/bin/go test','go test')+' && node --test probe.test.cjs'
        shim_env=dict(env,PATH=f'{shim}:{env["PATH"]}')
        def compound_case(name,route,first_fails=False,second_fails=False,uncertain=False):
            payload=compound_inner
            if first_fails or second_fails or uncertain:
                payload="export FAIL_FIXTURE=%d NODE_FAIL=%d; "%(first_fails,second_fails)+payload
                payload="printf '%s\\n' \"$RCH_FIXTURE_TRANSPORT\" >> shell-runs; printf 'owned\\000\\376' >&2; "+payload
            source='rtk proxy sh -c '+shlex.quote(payload)
            before=len(selections);before_release=len(releases)
            go_before=(cwd/'literal-runs').read_bytes()
            node_before=(cwd/'node-runs').read_bytes() if (cwd/'node-runs').exists() else b''
            shell_before=(cwd/'shell-runs').read_bytes() if (cwd/'shell-runs').exists() else b''
            event={'hook_event_name':'PreToolUse','tool_name':'Bash','tool_input':{'command':source},'cwd':str(cwd)}
            hook=subprocess.run([str(root/'rch'),'--no-self-healing'],input=json.dumps(event).encode(),env=shim_env,cwd=cwd,capture_output=True,timeout=10)
            assert hook.returncode==0 and not hook.stderr,(hook.returncode,hook.stderr)
            updated=json.loads(hook.stdout)['hookSpecificOutput']['updatedInput']['command']
            assert updated=='rch exec --verification --require-remote -- '+source,updated
            assert len(selections)==before and len(releases)==before_release
            run=subprocess.run(['/bin/sh','-c',updated if route=='hook' else source],env=dict(shim_env,**({'MISSING_RECEIPT':'1'} if uncertain else {})),cwd=cwd,capture_output=True,timeout=120)
            (root/(name+'.stdout')).write_bytes(run.stdout);(root/(name+'.stderr')).write_bytes(run.stderr)
            expected=int(first_fails or second_fails or uncertain)
            assert run.returncode==expected,(name,run.returncode,run.stdout,run.stderr)
            assert len(selections)==before+1 and 'rtk proxy' in selections[-1]['command'][0],selections[-1]
            assert (cwd/'literal-runs').read_bytes()==go_before+b'x'
            node_after=(cwd/'node-runs').read_bytes() if (cwd/'node-runs').exists() else b''
            assert node_after==node_before+(b'' if first_fails else b'x'),(name,node_after)
            resolved=json.loads((cwd/'resolved.json').read_text())
            assert resolved['transport']=='worker' and resolved['literal']=='literal value ; $HOME 中文',resolved
            assert resolved['cwd']==str(cwd) and resolved['GOCACHE']==str(root/'worker-cache/go-build'),resolved
            assert resolved['GOMODCACHE']==str(root/'worker-cache/go-mod') and resolved['tmp'].startswith(str(root/'tmp')),resolved
            if first_fails or second_fails or uncertain:
                assert (cwd/'shell-runs').read_bytes()==shell_before+b'worker\n'
                assert b'owned\x00\xfe' in run.stderr,run.stderr
            lease=json.loads((root/'state/job-leases'/(selections[-1]['local_wrapper_id'][0]+'.json')).read_text())
            assert lease['terminal_acknowledged']==(not uncertain),lease
            assert len(releases)==before_release+(0 if uncertain else 1),releases[-1]
            if not uncertain:
                assert lease['exit_code']==expected and releases[-1]['exit_code']==[str(expected)],lease
                assert lease['recovery']['sources_released'] and lease['recovery']['retired'],lease
                assert not pathlib.Path(resolved['tmp']).exists(),resolved
                logs=pathlib.Path(lease['recovery']['rtk_output']['local'])
                assert (logs/'outer.stdout').read_bytes()==run.stdout
                assert (logs/'outer.stderr').read_bytes()==(b'owned\x00\xfe' if first_fails or second_fails else b'')
                assert b'RAW-LINE-000' in run.stdout
                assert (b'NODE-COMPOUND-RAW' in run.stdout)==(not first_fails)
            (root/(name+'.json')).write_text(json.dumps({'source':source,'rewrite':updated,'route':route,'selection':selections[-1],'lease':lease,'resolved':resolved,'exit_code':run.returncode,'go_delta':1,'node_delta':int(not first_fails),'uncertain':uncertain},ensure_ascii=False,indent=2))
            print(name,'PASS','one outer selection','exit',run.returncode,'no replay')
        compound_case('compound-hook','hook')
        compound_case('compound-shim','shim')
        compound_case('compound-go-failure','shim',first_fails=True)
        compound_case('compound-node-failure','hook',second_fails=True)
        # The live 328-byte runpy wrapper acquired E2BIG only after nested
        # shell generation. Adapt its fixture operand/cwd, never live inputs.
        raw_work=root/'source/raw-launch';raw_work.mkdir()
        cancel_source=raw_work/'cancel.py'
        cancel_source.write_text('''import json,os,pathlib,subprocess,time
assert os.environ['RCH_FIXTURE_TRANSPORT']=='worker'
child=subprocess.Popen(['/usr/bin/python3','-c','import time;time.sleep(60)'])
pathlib.Path('pids.json').write_text(json.dumps([os.getpid(),child.pid]))
with open('runs','ab') as f:f.write(b'x')
os.write(1,b'pre-cancel-out\\x00\\xff');os.write(2,b'pre-cancel-err\\x00\\xfe')
time.sleep(60)
''')
        wrapper="import os,json,pathlib,runpy,socket; pathlib.Path('storage-env.json').write_text(json.dumps({'runtime_host':socket.gethostname(),'HOME':os.environ.get('HOME'),'TMPDIR':os.environ.get('TMPDIR')})); runpy.run_path(%r,run_name='__main__')"%str(cancel_source)
        for name,expected in [('raw-runpy-interrupt',130),('raw-runpy-deadline',124)]:
            work=raw_work/name;work.mkdir();result=root/(name+'.json')
            before=len(selections);before_release=len(releases)
            args=[str(root/'rch'),'--no-self-healing','exec','--verification','--workspace','shared','--cwd',str(work),'--timeout','30s' if expected==130 else '2s','--json-result',str(result),'--','rtk','proxy','/usr/bin/python3','-c',wrapper]
            p=subprocess.Popen(args,env=env,cwd=work,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            started=time.monotonic()
            while not (work/'pids.json').exists():
                assert p.poll() is None and time.monotonic()-started<10
                time.sleep(.02)
            if expected==130:
                time.sleep(.1);p.send_signal(signal.SIGINT)
            out,err=p.communicate(timeout=20)
            (root/(name+'.stdout')).write_bytes(out);(root/(name+'.stderr')).write_bytes(err)
            assert p.returncode==expected,(p.returncode,out,err)
            record=json.loads(result.read_text());assert record['outcome']==('interrupted' if expected==130 else 'deadline_exceeded'),record
            assert out==b'pre-cancel-out\x00\xff' and b'pre-cancel-err\x00\xfe' in err
            assert len(selections)==before+1 and len(releases)==before_release+1 and releases[-1]['exit_code']==[str(expected)]
            lease=json.loads((root/'state/job-leases'/(selections[-1]['local_wrapper_id'][0]+'.json')).read_text())
            assert lease['terminal_acknowledged'] and lease['exit_code']==expected and lease['recovery']['retired'],lease
            witness=json.loads((work/'storage-env.json').read_text())
            assert witness['HOME']==str(root/'worker-home') and witness['TMPDIR'].startswith(str(root/'tmp')),witness
            assert not pathlib.Path(witness['TMPDIR']).exists()
            for pid in json.loads((work/'pids.json').read_text()):
                stat=pathlib.Path('/proc')/str(pid)/'stat'
                assert not stat.exists() or stat.read_text().rsplit(') ',1)[1][0] in 'ZX',pid
            assert (work/'runs').read_bytes()==b'x'
            logs=pathlib.Path(lease['recovery']['rtk_output']['local'])
            assert (logs/'outer.stdout').read_bytes()==out and (logs/'outer.stderr').read_bytes()==b'pre-cancel-err\x00\xfe'
            launch=pathlib.Path(lease['recovery']['completion']+'.launch')
            assert launch.is_file() and launch.stat().st_size<=120*1024
            (root/(name+'-evidence.json')).write_text(json.dumps({'argv':args,'witness':witness,'lease':lease,'launch_bytes':launch.stat().st_size,'release':releases[-1]},indent=2))
            print(name,'PASS','once-only','exit',expected,'staged bytes',launch.stat().st_size)
        # Staging must not consume any workload stdin, and shell/argv data must
        # survive the extra transport without being expanded or recompressed.
        result=root/'raw-stdin.json';payload=b'input\0\xff\n'*8192
        literal="literal ' value ; $HOME 中文"
        code_in="import os,sys; assert os.environ['LITERAL']==sys.argv[1]; sys.stdout.buffer.write(sys.stdin.buffer.read()); os.write(2,b'error\\x00\\xfe'); sys.exit(23)"
        run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(raw_work),'--json-result',str(result),'--env','LITERAL='+literal,'--','rtk','proxy','/usr/bin/python3','-c',code_in,literal],env=env,cwd=raw_work,input=payload,capture_output=True,timeout=30)
        assert run.returncode==23 and run.stdout==payload,(run.returncode,run.stderr)
        record=json.loads(result.read_text());logs=pathlib.Path(record['result_dirs'][0]['path'])
        assert (logs/'outer.stdout').read_bytes()==payload and (logs/'outer.stderr').read_bytes()==b'error\x00\xfe'
        print('raw-stdin-argv-env PASS',len(payload),'bytes exit23')
        # Generated-size refusal is definite preparation failure. No workload
        # transport is launched, and source/slot ownership retires immediately.
        for name,extra in [('raw-size-refusal',"; #"+"'"*2048)]:
            result=root/(name+'.json');before=len(selections);before_release=len(releases)
            launches=(root/'raw-launches.jsonl').read_bytes()
            run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(raw_work),'--json-result',str(result),'--','rtk','proxy','/usr/bin/python3','-c',"open('must-not-run','w').write('bad')"+extra],env=env,cwd=raw_work,capture_output=True,timeout=30)
            assert run.returncode==1 and b'supported limit is 122880 bytes; workload not started' in run.stderr,run.stderr
            assert not (raw_work/'must-not-run').exists() and (root/'raw-launches.jsonl').read_bytes()==launches
            assert len(selections)==before+1 and len(releases)==before_release+1 and releases[-1]['exit_code']==['1']
            lease=json.loads((root/'state/job-leases'/(selections[-1]['local_wrapper_id'][0]+'.json')).read_text())
            assert lease['terminal_acknowledged'] and lease['exit_code']==1 and lease['state']=='finished',lease
            assert not lease['recovery']['execution_started'] and lease['recovery']['preparation_cancelled'] and lease['recovery']['sources_released'] and lease['recovery']['retired'],lease
            (root/(name+'-evidence.json')).write_text(json.dumps({'lease':lease,'release':releases[-1],'stderr':run.stderr.decode()},indent=2))
            print(name,'PASS','not started, source and own reservation retired')
        # Immediate reuse proves the refused job did not leave a source claim.
        run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(raw_work),'--','/usr/bin/printf','after-refusal'],env=env,cwd=raw_work,capture_output=True,timeout=15)
        assert run.returncode==0 and run.stdout==b'after-refusal',(run.returncode,run.stderr)
        # Even an SSH that exits before payload cannot prove unstarted to RCH.
        # Run in a disjoint source so its intentionally retained claim is safe.
        work=root/'source/raw-post-spawn';work.mkdir();result=root/'raw-post-spawn.json'
        before=len(selections);before_release=len(releases)
        run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(work),'--json-result',str(result),'--','rtk','proxy','/usr/bin/printf','unreachable'],env=dict(env,FAIL_AFTER_RAW_SPAWN='1'),cwd=work,capture_output=True,timeout=20)
        assert run.returncode==1 and json.loads(result.read_text())['outcome']=='transport_error',run.stderr
        assert len(selections)==before+1 and len(releases)==before_release
        lease=json.loads((root/'state/job-leases'/(selections[-1]['local_wrapper_id'][0]+'.json')).read_text())
        assert lease['recovery']['execution_started'] and not lease['terminal_acknowledged'],lease
        (root/'raw-post-spawn-evidence.json').write_text(json.dumps(lease,indent=2))
        print('raw-post-spawn-uncertain PASS','ownership retained, no replay')
        # RTK passthrough deadlines retain byte-exact pre-cancellation output,
        # and the existing process-group cleanup must include captured tools.
        for name,expected,outcome in [('rtk-timeout',124,'deadline_exceeded'),('rtk-interrupt',130,'interrupted')]:
            work=cwd/name;work.mkdir();result=root/(name+'.json')
            args=[str(root/'rch'),'--no-self-healing','exec','--verification','--cwd',str(work),'--json-result',str(result),'--timeout','2s' if expected==124 else '20s','--','rtk','proxy','/usr/bin/python3','-c',code]
            started=time.monotonic();p=subprocess.Popen(args,env=env,cwd=work,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            while not (work/'pids').exists():
                assert p.poll() is None and time.monotonic()-started<10
                time.sleep(.02)
            if expected==130:
                time.sleep(.1);p.send_signal(signal.SIGINT)
            stdout,stderr=p.communicate(timeout=20);record=json.loads(result.read_text())
            assert p.returncode==expected and record['outcome']==outcome,(p.returncode,record,stderr)
            assert time.monotonic()-started<10 and releases[-1]['exit_code']==[str(expected)]
            logs=pathlib.Path(record['result_dirs'][0]['path'])
            assert stdout==b'out\x00\xff' and (logs/'outer.stdout').read_bytes()==stdout
            assert b'err\x00\xfe' in (logs/'outer.stderr').read_bytes()
            for pid in (work/'pids').read_text().split():
                stat=pathlib.Path('/proc')/pid/'stat'
                assert not stat.exists() or stat.read_text().rsplit(') ',1)[1][0] in 'ZX',pid
            assert (work/'runs').read_bytes()==b'x'
            print(name,'PASS',round(time.monotonic()-started,3))
        # A completed command whose log transfer failed releases its slot and
        # is collected by jobs recover without re-execution or a new selection.
        result=root/'rtk-recover.json';before=len(selections)
        run=subprocess.run([str(root/'rch'),'--no-self-healing','exec','--verification','--json-result',str(result),'--','rtk','run','-c','printf retained; printf x >> capture-runs; exit 7'],env=dict(env,FAIL_RTK_COLLECTION='1'),cwd=cwd,capture_output=True,timeout=30)
        assert run.returncode==102,(run.returncode,run.stderr)
        assert len(selections)==before+1 and releases[-1]['exit_code']==['102']
        wrapper=selections[-1]['local_wrapper_id'][0]
        recovered=subprocess.run([str(root/'rch'),'--no-self-healing','jobs','recover',wrapper],env=env,cwd=cwd,capture_output=True,timeout=30)
        assert b'full RTK output:' in recovered.stderr,(recovered.returncode,recovered.stdout,recovered.stderr)
        lease=json.loads((root/'state/job-leases'/(wrapper+'.json')).read_text())
        assert lease['terminal_acknowledged'] and lease['recovery']['rtk_output']['collected'],lease
        assert (pathlib.Path(lease['recovery']['rtk_output']['local'])/'outer.stdout').read_bytes()==b'retained'
        assert (cwd/'capture-runs').read_bytes()==b'x' and len(selections)==before+1
        print('rtk-output-recovery-no-replay PASS')
        # Missing completion deliberately retains the source claim. Run last
        # so later jobs do not contend with this intentionally unresolved owner.
        compound_case('compound-uncertain','shim',uncertain=True)
    assert not errors,errors
finally:
    done=True; thread.join(timeout=3); listener.close()
    (root/'requests.json').write_text(json.dumps(requests,indent=2))
print('evidence:',root)
"#)
        .arg(&root)
        .arg(std::env::var("RCH_TEST_NATIVE_RUST_BIN").expect("set RCH_TEST_NATIVE_RUST_BIN to an installed stable native toolchain bin directory"))
        .output().unwrap();
    assert!(
        output.status.success(),
        "evidence={}\n{}\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stdout));
}

struct Fixture {
    root: TempDir,
    lease: DurableJobLease,
    lease_path: PathBuf,
    socket: PathBuf,
}

impl Fixture {
    fn new(admitted: bool) -> Self {
        let root = TempDir::new().unwrap();
        let state = root.path().join("state");
        fs::create_dir_all(state.join("job-leases")).unwrap();
        fs::create_dir_all(root.path().join("home")).unwrap();
        fs::create_dir_all(root.path().join("config")).unwrap();
        let pid = std::process::id();
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let ticks = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        let mut lease = DurableJobLease::new(
            JobIdentity::new_local(),
            pid,
            Some(ticks),
            Some(boot.trim().to_owned()),
            0,
            true,
            false,
            "test-only-command-fingerprint".into(),
        );
        if admitted {
            lease.admit(42, "worker-a".into(), 1);
        }
        let lease_path = state
            .join("job-leases")
            .join(format!("{}.json", lease.identity.local_wrapper_id));
        write_lease(&lease_path, &lease);
        let socket = root.path().join("daemon.sock");
        Self {
            root,
            lease,
            lease_path,
            socket,
        }
    }

    fn spawn(&self, action: &str, timeout_secs: Option<u64>) -> OwnedChild {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rch"));
        command
            .env_clear()
            .env("HOME", self.root.path().join("home"))
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("RCH_STATE_HOME", self.root.path().join("state"))
            .env("RCH_SOCKET_PATH", &self.socket)
            .env("RCH_LOG_LEVEL", "error")
            .env("NO_COLOR", "1")
            .current_dir(self.root.path())
            .args([
                "--json",
                "jobs",
                action,
                &self.lease.identity.local_wrapper_id,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(timeout_secs) = timeout_secs {
            command.args(["--timeout-secs", &timeout_secs.to_string()]);
        }
        OwnedChild(Some(command.spawn().unwrap()))
    }

    fn cancel_receipt(&self) -> PathBuf {
        self.lease_path
            .with_file_name(format!("{}.cancel", self.lease.identity.local_wrapper_id))
    }

    fn active(&self) -> Value {
        json!({
            "status": "active",
            "active": {
                "id": 42,
                "local_wrapper_id": self.lease.identity.local_wrapper_id,
                "worker_id": "worker-a",
            },
        })
    }

    fn completed(&self, code: i32) -> Value {
        json!({
            "status": "completed",
            "local_wrapper_id": self.lease.identity.local_wrapper_id,
            "record": {"id": 42, "worker_id": "worker-a", "exit_code": code},
        })
    }
}

fn write_lease(path: &Path, lease: &DurableJobLease) {
    let staged = path.with_extension("pending");
    fs::write(&staged, serde_json::to_vec(lease).unwrap()).unwrap();
    fs::rename(staged, path).unwrap();
}

/// Reap only the child this test created, including on assertion failure.
struct OwnedChild(Option<Child>);

impl OwnedChild {
    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.0.take().unwrap().wait_with_output().unwrap();
            }
            assert!(Instant::now() < deadline, "jobs CLI did not terminate");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn peer(
    socket: &Path,
    replies: Vec<Value>,
    after_request: impl Fn(usize) + Send + 'static,
) -> JoinHandle<Vec<String>> {
    let listener = UnixListener::bind(socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut requests = Vec::new();
        for (index, reply) in replies.into_iter().enumerate() {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "expected daemon request missing");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = String::new();
            // The production helper half-closes its request before reading.
            Read::by_ref(&mut stream)
                .take(8192)
                .read_to_string(&mut request)
                .unwrap();
            requests.push(request);
            after_request(index);
            write!(
                stream,
                "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{reply}\n"
            )
            .unwrap();
        }
        requests
    })
}

fn output_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value.get("data").cloned().unwrap_or(value)
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn queued_attach_waits_for_its_budget_without_querying_a_nonexistent_build() {
    let fixture = Fixture::new(false);
    let before = fs::read(&fixture.lease_path).unwrap();
    let started = Instant::now();
    let output = fixture.spawn("attach", Some(1)).finish();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("same-id job is still pending"));
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(fs::read(&fixture.lease_path).unwrap(), before);
    assert!(!fixture.cancel_receipt().exists());
}

#[test]
fn attach_follows_admission_and_the_original_terminal_acknowledgement() {
    let fixture = Fixture::new(false);
    let path = fixture.lease_path.clone();
    let mut terminal = fixture.lease.clone();
    terminal.admit(42, "worker-a".into(), 1);
    terminal.exit_code = Some(101);
    terminal.acknowledge_terminal(2);
    let daemon = peer(&fixture.socket, vec![fixture.active()], move |_| {
        write_lease(&path, &terminal);
    });
    let child = fixture.spawn("attach", Some(5));
    thread::sleep(Duration::from_millis(150));
    let mut admitted = fixture.lease.clone();
    admitted.admit(42, "worker-a".into(), 1);
    write_lease(&fixture.lease_path, &admitted);
    let result = output_json(&child.finish());
    let requests = daemon.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /builds/42?local_wrapper_id="));
    assert_eq!(
        result["identity"]["local_wrapper_id"],
        fixture.lease.identity.local_wrapper_id
    );
    assert_eq!(result["exit_code"], 101);
    assert_eq!(result["terminal_acknowledged"], true);
    assert!(!fixture.cancel_receipt().exists());
}

#[test]
fn cancellation_losing_to_completion_preserves_the_owner_journal() {
    let mut fixture = Fixture::new(true);
    fixture.lease.recovery = Some(json!({"retired": false}));
    write_lease(&fixture.lease_path, &fixture.lease);
    let before = fs::read(&fixture.lease_path).unwrap();
    let daemon = peer(
        &fixture.socket,
        vec![fixture.active(), fixture.completed(101)],
        |_| {},
    );
    let result = output_json(&fixture.spawn("cancel", None).finish());
    let requests = daemon.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].starts_with("POST /builds/42/cancel?local_wrapper_id="));
    assert_eq!(result["status"], "completed");
    assert_eq!(result["exit_code"], 101);
    assert_eq!(result["terminal_acknowledged"], false);
    assert_eq!(result["has_recovery_journal"], true);
    assert_eq!(result["wrapper_stop_requested"], false);
    assert!(!fixture.cancel_receipt().exists());
    assert_eq!(fs::read(&fixture.lease_path).unwrap(), before);
}

#[test]
fn cancelling_an_already_completed_job_is_a_non_mutating_noop() {
    let fixture = Fixture::new(true);
    let before = fs::read(&fixture.lease_path).unwrap();
    let daemon = peer(&fixture.socket, vec![fixture.completed(0)], |_| {});
    let result = output_json(&fixture.spawn("cancel", None).finish());
    let requests = daemon.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /builds/42?local_wrapper_id="));
    assert_eq!(result["status"], "completed");
    assert_eq!(result["wrapper_stop_requested"], false);
    assert_eq!(result["terminal_acknowledged"], false);
    assert!(!fixture.cancel_receipt().exists());
    assert_eq!(fs::read(&fixture.lease_path).unwrap(), before);
}

#[test]
fn foreign_cancellation_receipt_never_requests_a_wrapper_stop() {
    let fixture = Fixture::new(true);
    let before = fs::read(&fixture.lease_path).unwrap();
    let daemon = peer(
        &fixture.socket,
        vec![
            fixture.active(),
            json!({"status": "cancelled", "build_id": 43, "worker_id": "worker-a"}),
        ],
        |_| {},
    );
    let output = fixture.spawn("cancel", None).finish();
    daemon.join().unwrap();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("cancellation build/worker mismatch"));
    assert!(!fixture.cancel_receipt().exists());
    assert_eq!(fs::read(&fixture.lease_path).unwrap(), before);
}

#[test]
fn observing_completion_cannot_overwrite_a_live_owners_newer_recovery_journal() {
    let fixture = Fixture::new(true);
    let path = fixture.lease_path.clone();
    let mut collecting = fixture.lease.clone();
    collecting.recovery = Some(json!({"retired": false, "phase": "sync_down"}));
    let daemon = peer(
        &fixture.socket,
        vec![fixture.completed(0), fixture.completed(0)],
        move |index| {
            if index == 0 {
                // Publish a newer journal after attach took its no-recovery
                // snapshot, but before it consumes the completion response.
                write_lease(&path, &collecting);
            } else {
                let retained: DurableJobLease =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert_eq!(retained.recovery, collecting.recovery);
                assert!(!retained.terminal_acknowledged);
                let mut terminal = collecting.clone();
                terminal.exit_code = Some(0);
                terminal.acknowledge_terminal(3);
                write_lease(&path, &terminal);
            }
        },
    );
    let result = output_json(&fixture.spawn("attach", Some(5)).finish());
    let requests = daemon.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("GET /builds/42?"))
    );
    assert_eq!(result["terminal_acknowledged"], true);
    assert!(!fixture.cancel_receipt().exists());
}

#[test]
fn matching_cancellation_requests_only_the_original_wrapper() {
    let fixture = Fixture::new(true);
    let before = fs::read(&fixture.lease_path).unwrap();
    let daemon = peer(
        &fixture.socket,
        vec![
            fixture.active(),
            json!({"status": "cancelled", "build_id": 42, "worker_id": "worker-a"}),
        ],
        |_| {},
    );
    let result = output_json(&fixture.spawn("cancel", None).finish());
    daemon.join().unwrap();
    assert_eq!(result["wrapper_stop_requested"], true);
    let receipt: JobIdentity =
        serde_json::from_slice(&fs::read(fixture.cancel_receipt()).unwrap()).unwrap();
    assert_eq!(receipt, fixture.lease.identity);
    assert_eq!(fs::read(&fixture.lease_path).unwrap(), before);
}

#[test]
fn owner_exit_during_status_does_not_erase_a_new_recovery_intent() {
    let mut fixture = Fixture::new(true);
    // Reap a process this fixture actually owns. Out-of-range synthetic PIDs
    // are unknown identities and cannot authorize completion reconciliation.
    let owner = OwnedChild(Some(
        Command::new("/bin/true")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let owner_pid = owner.0.as_ref().unwrap().id();
    assert!(owner.finish().status.success());
    assert_eq!(
        rch_common::process_identity::owner_presence(owner_pid, None),
        rch_common::process_identity::OwnerPresence::Absent
    );
    fixture.lease.wrapper_pid = owner_pid;
    write_lease(&fixture.lease_path, &fixture.lease);
    let path = fixture.lease_path.clone();
    let mut latest = fixture.lease.clone();
    latest.recovery = Some(json!({"retired": false, "phase": "sync_down"}));
    let retained = serde_json::to_vec(&latest).unwrap();
    let daemon = peer(
        &fixture.socket,
        vec![fixture.completed(0), fixture.completed(0)],
        move |index| {
            if index == 0 {
                write_lease(&path, &latest);
            }
        },
    );
    let output = fixture.spawn("attach", Some(5)).finish();
    daemon.join().unwrap();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("use jobs recover"));
    assert_eq!(fs::read(&fixture.lease_path).unwrap(), retained);
    assert!(!fixture.cancel_receipt().exists());
}
