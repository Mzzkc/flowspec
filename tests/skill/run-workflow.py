#!/usr/bin/env python3
"""Exercise the exact extracted binary. Any HOLD is a failed qualification."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import zipfile

ROOT=Path(__file__).resolve().parents[2]
FIXTURES=Path(__file__).resolve().parent/'fixtures'
MEMBERS={
 'flowspec/SKILL.md','flowspec/references/report-contract.md',
 'flowspec/references/examples.md','flowspec/LICENSE','flowspec/LICENSE-AGPL',
 'flowspec/bin/flowspec','flowspec/package.json','flowspec/SHA256SUMS',
}


def digest(data):return hashlib.sha256(data).hexdigest()


def extract(archive,dest):
    with zipfile.ZipFile(archive) as z:
        if len(z.infolist())!=len(MEMBERS) or set(z.namelist())!=MEMBERS:
            raise ValueError('archive allowlist mismatch or duplicate members')
        if sum(i.file_size for i in z.infolist())>260*1024*1024:
            raise ValueError('oversize archive')
        for info in z.infolist():
            mode=info.external_attr>>16
            if stat.S_ISLNK(mode) or not stat.S_ISREG(mode):
                raise ValueError('unsafe archive member type')
        rows=z.read('flowspec/SHA256SUMS').decode().splitlines()
        sums={}
        for row in rows:
            sha,name=row.split('  ')
            if name in sums or 'flowspec/'+name not in MEMBERS:
                raise ValueError('unsafe or duplicate checksum member')
            sums[name]=sha
        if set(sums)!={m.removeprefix('flowspec/') for m in MEMBERS if m!='flowspec/SHA256SUMS'}:
            raise ValueError('checksum coverage mismatch')
        for name,sha in sums.items():
            if digest(z.read('flowspec/'+name))!=sha:raise ValueError('archive checksum mismatch: '+name)
        manifest=json.loads(z.read('flowspec/package.json'))
        if digest(z.read('flowspec/bin/flowspec'))!=manifest['binary_sha256']:
            raise ValueError('binary manifest mismatch')
        if manifest['schema']!='flowspec.skill-package.v1':raise ValueError('unknown package schema')
        for info in z.infolist():
            path=dest/info.filename
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes(z.read(info.filename))
            path.chmod(0o755 if info.filename.endswith('/bin/flowspec') else 0o644)
    return dest/'flowspec/bin/flowspec',manifest


def run_workflow(archive,out):
    archive,out=Path(archive).resolve(),Path(out).resolve()
    out.mkdir(parents=True,exist_ok=True)
    result={'schema':'flowspec.skill-workflow.v1','archive':str(archive),'archive_sha256':digest(archive.read_bytes()),'import_path':str(Path(__file__).resolve()),'earned':[],'holds':[],'commands':[], 'release_qualified':False, 'fixture_digests':{p.name:digest(p.read_bytes()) for p in sorted(FIXTURES.glob('*.py'))}}
    def record(name,passed,observed,expected):
        if passed:result['earned'].append(name)
        else:result['holds'].append({'operation':name,'status':'HOLD','expected':expected,'observed':observed})
    with tempfile.TemporaryDirectory(prefix='flowspec-skill-workflow-') as t:
        sandbox=Path(t);binary,manifest=extract(archive,sandbox)
        result['package_manifest']=manifest
        result['binary_sha256']=digest(binary.read_bytes())
        home=sandbox/'empty-home';home.mkdir()
        env={'HOME':str(home),'PATH':os.defpath,'LC_ALL':'C','TZ':'UTC'}
        def invoke(label,args):
            r=subprocess.run([str(binary),*map(str,args)],cwd=sandbox,env=env,capture_output=True,timeout=30)
            stdout=out/(label+'.stdout');stderr=out/(label+'.stderr')
            stdout.write_bytes(r.stdout);stderr.write_bytes(r.stderr)
            result['commands'].append({'label':label,'argv':[str(binary),*map(str,args)],'exit':r.returncode,'stdout':str(stdout),'stderr':str(stderr),'stdout_sha256':digest(r.stdout),'stderr_sha256':digest(r.stderr)})
            try:value=json.loads(r.stdout)
            except (ValueError,UnicodeError):value=None
            return r.returncode,value,r.stdout.decode(errors='replace')
        version=invoke('version',['--version'])
        result['version']=version[2].strip()
        scopes={}
        for kind in ['clean','findings','incomplete','trace']:
            scope=sandbox/kind;scope.mkdir();(scope/'main.py').write_bytes((FIXTURES/(kind+'.py')).read_bytes());scopes[kind]=scope
        report_args=['--format','json','--quiet','--gate-severity','info','--gate-confidence','low']
        def analyze(kind,label=None,extra=()):
            return invoke(label or kind,['analyze',scopes[kind],*report_args,*extra])
        clean_code,clean,_=analyze('clean')
        a=clean.get('authority',{}) if isinstance(clean,dict) else {}
        record('clean_exit_0',clean_code==0 and a.get('execution')=='complete' and a.get('outcome')=='clean' and a.get('findings',{}).get('total')==0,{'exit':clean_code,'execution':a.get('execution'),'outcome':a.get('outcome')},'exit 0, complete clean report, zero full findings')
        result['observed_producer']=a.get('producer')
        if a.get('producer')!=manifest['binary_producer']:raise ValueError('workflow producer differs from package-bound binary')
        error_code,error,_=invoke('error',['analyze',sandbox/'missing',*report_args])
        record('error_exit_1',error_code==1 and error is None,{'exit':error_code,'report':error},'exit 1; no contract-valid report for missing path')
        f_code,findings,_=analyze('findings')
        fa=findings.get('authority',{}) if isinstance(findings,dict) else {}
        found=fa.get('findings',{}).get('all',[])
        record('findings_exit_2',f_code==2 and fa.get('execution')=='complete' and fa.get('outcome')=='findings' and len(found)==1,{'exit':f_code,'execution':fa.get('execution'),'outcome':fa.get('outcome'),'count':len(found)},'exit 2 complete findings report with one private-symbol finding')
        i_code,incomplete,_=analyze('incomplete')
        ia=incomplete.get('authority',{}) if isinstance(incomplete,dict) else {}
        record('incomplete_exit_3',i_code==3 and ia.get('execution')=='incomplete' and ia.get('outcome')=='incomplete' and bool(ia.get('losses')),{'exit':i_code,'execution':ia.get('execution'),'outcome':ia.get('outcome'),'losses':ia.get('losses')},'exit 3 with explicit selected parse loss')
        d_code,diagnosis,_=invoke('diagnose',['diagnose',scopes['findings'],*report_args])
        da=diagnosis.get('authority',{}) if isinstance(diagnosis,dict) else {}
        exact=da.get('findings',{}).get('all',[])
        expected_range={'start_byte':0,'end_byte':27,'start_line':1,'end_line':2,'start_column':1,'end_column':13,'path_key':'6d61696e2e7079'}
        expected_id='sha256:fe11a5179a94c2a3c0b20f305fd4cf913c254d1d514beb4f4b93a3e3527e49af'
        record('diagnose_exact',d_code==2 and len(exact)==1 and exact==found and exact[0].get('pattern_id')=='data_dead_end' and exact[0].get('subject',{}).get('qualified_name')=='main.py::_unused' and exact[0].get('range')==expected_range and exact[0].get('semantic_id')==expected_id and exact[0].get('evidence')==[{'kind':'graph_inbound_consumption','observation':'0 callers in 1 analyzed files','range':None}],{'exit':d_code,'findings':exact},'exact private symbol/range/stable identity/evidence, no extra finding')
        tr_code,trace,_=invoke('trace',['trace',scopes['trace'],'--symbol','main.py::main','--direction','forward','--format','json','--quiet'])
        paths=[{'entry':p.get('entry'),'exit':p.get('exit'),'steps':[(s.get('entity'),s.get('action')) for s in p.get('steps',[])]} for p in trace] if isinstance(trace,list) else None
        expected_paths=[{'entry':'main.py::main','exit':'main.py::leaf','steps':[('main.py::leaf','call')]}]
        record('trace_exact',tr_code==0 and paths==expected_paths,{'exit':tr_code,'paths':paths},'one exact ordered main-to-leaf call path and no extras')
        # Report files live outside the selected source so they cannot perturb
        # discovery/input identity during the before/after loop.
        before=sandbox/'before.json';after=sandbox/'after.json';bad=sandbox/'incomplete.json'
        before.write_text(json.dumps(clean));bad.write_text(json.dumps(incomplete))
        (scopes['clean']/'main.py').write_text('# changed clean module\n')
        after_code,after_value,_=analyze('clean','after');after.write_text(json.dumps(after_value))
        diff_code,diff,_=invoke('diff-comparable',['diff',before,after,'--format','json','--quiet'])
        lists=['entities_added','entities_removed','entities_changed','diagnostics_new','diagnostics_resolved']
        record('comparable_diff',after_code==0 and diff_code==0 and isinstance(diff,dict) and all(diff.get(k)==[] for k in lists) and diff.get('has_regressions') is False,{'exit':diff_code,'diff':diff},'complete same-scope comment edit comparable, no structural changes')
        bad_code,bad_diff,_=invoke('diff-incomplete',['diff',before,bad,'--format','json','--quiet'])
        record('incomparable_diff',bad_code==3 and (not isinstance(bad_diff,dict) or bad_diff.get('diagnostics_resolved',[])==[]),{'exit':bad_code,'diff':bad_diff},'exit 3 for incomplete input; never assert resolution')
        sarif_code,sarif,_=invoke('sarif-incomplete',['analyze',scopes['incomplete'],'--format','sarif','--quiet'])
        runs=sarif.get('runs',[]) if isinstance(sarif,dict) else []
        inv=runs[0].get('invocations',[]) if len(runs)==1 else []
        record('sarif_incomplete',sarif_code==3 and len(inv)==1 and inv[0].get('executionSuccessful') is False and bool(inv[0].get('toolExecutionNotifications')),{'exit':sarif_code,'invocations':inv},'exit 3; unsuccessful SARIF invocation with visible loss notifications')
        continuation_code,page,_=analyze('findings','continuation',extra=['--limit','1','--offset','0'])
        ca=page.get('authority',{}) if isinstance(page,dict) else {}
        continuation=ca.get('continuation',{})
        record('continuation',continuation_code==2 and ca.get('findings',{}).get('total')==1 and continuation.get('shown')==1 and continuation.get('full')==1 and continuation.get('matched_total')==1 and continuation.get('truncated') is False and continuation.get('next_cursor') is None,{'exit':continuation_code,'metadata':continuation},'limit/offset accepted; full authority and exact terminal continuation retained')
        # Native handoff exercises a separate repository command against the
        # changed file, then checks the analyzer gate; FLOWSpec cannot replace it.
        validation=['python3','-m','py_compile',str(scopes['clean']/'main.py')]
        native=subprocess.run(validation,cwd=sandbox,env=env,capture_output=True,timeout=20)
        (out/'native-validation.stdout').write_bytes(native.stdout);(out/'native-validation.stderr').write_bytes(native.stderr)
        result['native_validation']={'argv':validation,'exit':native.returncode,'scope':'controlled Python fixture only; target repository still owns its real native suite'}
        gate_code,gate,_=invoke('structural-gate',['diagnose',scopes['clean'],'--gate-severity','info','--gate-confidence','low','--format','json','--quiet'])
        record('native_validation_handoff',native.returncode==0 and gate_code==0 and isinstance(gate,dict) and gate.get('authority',{}).get('execution')=='complete',{'native_exit':native.returncode,'structural_exit':gate_code},'native fixture validation followed by independent complete structural gate')
        result['support_limits']=[
            'Development archive is not a downloaded/reviewed release artifact.',
            'Trace currently emits a focused legacy path array rather than AnalysisReport authority.',
            'No CLI finding-ID diagnose selector is exposed; exact catalog matching is tested instead.',
            'Controlled Python fixtures do not qualify thirteen-by-four diagnostic semantics or target-repository validation.',
        ]
    result['status']='ACCEPT_DEVELOPMENTAL_WORKFLOW' if not result['holds'] else 'REWORK_WORKFLOW'
    (out/'workflow.json').write_text(json.dumps(result,sort_keys=True,indent=2)+'\n')
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--archive',type=Path,required=True);p.add_argument('--out',type=Path,required=True);args=p.parse_args()
    print('import:',Path(__file__).resolve(),file=sys.stderr)
    try:
        result=run_workflow(args.archive,args.out)
        print(json.dumps({'status':result['status'],'earned':result['earned'],'holds':result['holds']}))
        return 0 if not result['holds'] else 1
    except (ValueError,OSError,KeyError,subprocess.SubprocessError,zipfile.BadZipFile) as exc:
        args.out.mkdir(parents=True,exist_ok=True)
        result={'schema':'flowspec.skill-workflow.v1','status':'HOLD_HARNESS','earned':[],'holds':[{'operation':'harness','reason':str(exc)}],'release_qualified':False}
        (args.out/'workflow.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));return 1
if __name__=='__main__':sys.exit(main())
